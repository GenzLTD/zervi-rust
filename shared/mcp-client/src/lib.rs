//! # mcp-client —— 终端侧 MCP 客户端
//!
//! zervi-rust 作为 **MCP client**，把 cortex-mcp（Python）作为 **stdio 子进程** 拉起，
//! 随终端进程起停。定位见 `cortex-mcp接入任务清单-终端机-2026-10-17.md`。
//!
//! ## 当前进度：T1.0（握手探针）
//!
//! 本模块**刻意只做到握手**。原因：`rmcp`（Rust SDK）与 cortex-mcp 用的
//! `mcp`（Python SDK）是两套独立实现，包版本无法对齐，唯一契约是 `initialize`
//! 协商出的 `protocolVersion`。握手不通的话，后面所有工具调用代码都要推翻。
//!
//! 因此先提供 [`probe_handshake`]，让 `mcp-probe` 二进制能把
//! `protocolVersion` / `serverInfo` / 工具清单打出来，**再**往 [`McpMemoryClient`] 里填业务。
//!
//! ## 关于 lifecycle 模式（rmcp 3.5.x 的新东西）
//!
//! rmcp 3.x 除 legacy `initialize` 外，还支持新 spec 的 `server/discover`
//! （生命周期挪进 per-request `_meta`，`2026-07-28` 起不再用 `initialize`）。
//! 而 cortex-mcp 锁的是 `mcp>=1.2.0,<2`，**只认 `initialize`**。所以：
//!
//! - [`Lifecycle::Legacy`]（默认）—— 只发 `initialize`，与 Python 1.x 对齐，最稳；
//! - [`Lifecycle::Auto`] —— 先试 `discover`，对端报 legacy 或 10s 不响应则回退；
//! - [`Lifecycle::Discover`] —— 只发 `discover`，用于确认「对端到底支不支持新 spec」。
//!
//! 探针把三种都暴露出来，是为了让 T1.0 的结论建立在**实测**而不是推断上。

use std::path::PathBuf;

use anyhow::{Context, Result};
use rmcp::model::ProtocolVersion;
use rmcp::service::{ClientLifecycleMode, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ClientServiceExt;

/// 如何拉起 MCP 服务端子进程。
#[derive(Debug, Clone)]
pub struct McpServerSpec {
    /// 可执行文件，例如 `python3` / `python` / venv 里的绝对路径。
    pub program: String,
    /// 传参，cortex-mcp 用 `["-m", "cortex_mcp.server"]`。
    pub args: Vec<String>,
    /// 工作目录（cortex-mcp 读 `wukong-core/rules/` 相对路径时依赖它）。
    pub cwd: Option<PathBuf>,
    /// 额外环境变量（cortex-mcp 需要 `CORTEX_*` 那一组）。
    pub env: Vec<(String, String)>,
}

impl McpServerSpec {
    /// 以 `python -m <module>` 形式拉起。
    pub fn python_module(program: impl Into<String>, module: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: vec!["-m".into(), module.into()],
            cwd: None,
            env: Vec::new(),
        }
    }

    /// 追加一个环境变量（链式）。
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// 设置工作目录（链式）。
    pub fn with_cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    fn to_command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(&self.args);
        if let Some(dir) = &self.cwd {
            cmd.current_dir(dir);
        }
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        // stderr 保持 inherit（rmcp 的 builder 默认值）：
        // 握手失败时 Python 的 traceback 能直接出现在终端上，比抓进变量更好排查。
        cmd
    }
}

/// 生命周期协商模式，对应 rmcp 的 [`ClientLifecycleMode`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// 只发 legacy `initialize`。cortex-mcp（Python `mcp` 1.x）走这条。
    Legacy,
    /// 先试 `server/discover`，不支持则回退到 `initialize`。
    Auto,
    /// 只发 `server/discover`（新 spec），用于确认对端能力上限。
    Discover,
}

impl Lifecycle {
    /// 从命令行字符串解析（`legacy` / `auto` / `discover`）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "legacy" | "initialize" | "init" => Some(Lifecycle::Legacy),
            "auto" => Some(Lifecycle::Auto),
            "discover" => Some(Lifecycle::Discover),
            _ => None,
        }
    }

    fn to_mode(self) -> ClientLifecycleMode {
        // 客户端能说的版本上限：全部已知版本，由新到旧。
        let preferred = || ProtocolVersion::KNOWN_VERSIONS.iter().rev().cloned().collect::<Vec<_>>();
        match self {
            Lifecycle::Legacy => ClientLifecycleMode::Initialize,
            Lifecycle::Auto => ClientLifecycleMode::Auto {
                preferred_versions: preferred(),
                // 回退时锚定在「仍然走 initialize 的最高版本」，
                // 而不是无脑用 LATEST（LATEST 可能已是不走 initialize 的新 spec）。
                legacy_version: Some(ProtocolVersion::LATEST_WITH_INITIALIZE),
            },
            Lifecycle::Discover => ClientLifecycleMode::Discover {
                preferred_versions: preferred(),
            },
        }
    }
}

/// T1.0 探针的输出。
#[derive(Debug, Clone)]
pub struct HandshakeReport {
    /// 实际协商到的协议版本，例如 `"2025-06-18"`。
    pub protocol_version: String,
    /// 服务端自报名字，cortex-mcp 应为 `"cortex"`。
    pub server_name: String,
    /// 服务端自报版本。
    pub server_version: String,
    /// `tools/list` 拿到的工具名，cortex-mcp 应为 4 个。
    pub tools: Vec<String>,
    /// 实际使用的 lifecycle 模式（便于对拍时确认走的哪条路）。
    pub lifecycle: Lifecycle,
}

/// 拉起子进程、完成握手、列工具 —— **不调用任何业务工具**。
///
/// 这是 T1.0 的全部内容：只验「能不能对上话、对到哪个版本、它自报有哪些工具」。
pub async fn probe_handshake(spec: &McpServerSpec, lifecycle: Lifecycle) -> Result<HandshakeReport> {
    let transport = TokioChildProcess::new(spec.to_command()).with_context(|| {
        format!(
            "无法拉起 MCP 子进程：{} {}（检查 program 路径与 args）",
            spec.program,
            spec.args.join(" ")
        )
    })?;

    let service: RunningService<RoleClient, ()> = ()
        .serve_with_lifecycle(transport, lifecycle.to_mode())
        .await
        .with_context(|| {
            format!(
                "MCP 握手失败（lifecycle={:?}）。常见原因：① 对端只支持另一种 lifecycle；\
                 ② Python 环境缺依赖，子进程起不来；③ 对端 stdout 被日志污染，破坏了 JSON-RPC 帧。",
                lifecycle
            )
        })?;

    let peer_info = service
        .peer_info()
        .context("握手完成但拿不到 peer_info —— 这不该发生，请核对 rmcp 版本")?;

    let tools = service
        .peer()
        .list_all_tools()
        .await
        .context("tools/list 失败（握手已过，说明是工具层问题）")?
        .into_iter()
        .map(|t| t.name.to_string())
        .collect::<Vec<_>>();

    // `ServerPeerInfo::server_info` 在 rmcp 3.5.x 里是 Option ——
    // 走 `discover` 这条新路径时对端可能不自报 serverInfo，所以不能假设它一定在。
    let (server_name, server_version) = peer_info
        .server_info
        .as_ref()
        .map(|i| (i.name.clone(), i.version.clone()))
        .unwrap_or_else(|| ("(对端未自报 serverInfo)".to_string(), String::new()));

    let report = HandshakeReport {
        protocol_version: peer_info.protocol_version.as_str().to_string(),
        server_name,
        server_version,
        tools,
        lifecycle,
    };

    // 主动收尾，避免子进程泄漏。
    let _ = service.cancel().await;

    Ok(report)
}

/// 终端内嵌的记忆层客户端。
///
/// **当前只持句柄，业务方法（`recall` / `remember` / `forget` / `consolidate`）
/// 等 T1.0 实测通过后再填** —— 见模块文档。
pub struct McpMemoryClient {
    service: RunningService<RoleClient, ()>,
}

impl McpMemoryClient {
    /// 拉起并握手。
    pub async fn connect(spec: &McpServerSpec, lifecycle: Lifecycle) -> Result<Self> {
        let transport = TokioChildProcess::new(spec.to_command())
            .context("无法拉起 MCP 子进程（cortex-mcp）")?;
        let service = ()
            .serve_with_lifecycle(transport, lifecycle.to_mode())
            .await
            .context("MCP 握手失败（cortex-mcp）")?;
        Ok(Self { service })
    }

    /// 协商到的协议版本。
    pub fn protocol_version(&self) -> Option<String> {
        self.service
            .peer_info()
            .map(|i| i.protocol_version.as_str().to_string())
    }

    /// 服务端工具名清单（用于启动自检）。
    pub async fn list_tools(&self) -> Result<Vec<String>> {
        Ok(self
            .service
            .peer()
            .list_all_tools()
            .await
            .context("tools/list 失败")?
            .into_iter()
            .map(|t| t.name.to_string())
            .collect())
    }

    /// 优雅关闭（子进程随之退出）。
    pub async fn shutdown(self) -> Result<()> {
        self.service.cancel().await.context("关闭 MCP 客户端失败")?;
        Ok(())
    }
}
