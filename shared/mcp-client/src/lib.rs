//! # mcp-client —— 终端侧 MCP 客户端
//!
//! zervi-rust 作为 **MCP client**，把 cortex-mcp（Python）作为 **stdio 子进程** 拉起，
//! 随终端进程起停。定位见 `cortex-mcp接入任务清单-终端机-2026-10-17.md`。
//!
//! ## 进度
//!
//! - **T1.0（已完成）**：握手探针 [`probe_handshake`] + `mcp-probe` 二进制。
//!   Dell 夹具与 Mac 真机两处对拍结论一致：`Legacy` 可通过（协商到 `2025-11-25`），
//!   `Auto` 可通过但每次启动多一轮失败往返，`Discover` 被 Python `mcp` 1.x 判为
//!   `-32602` 而非 `UNSUPPORTED_PROTOCOL_VERSION` ⇒ **生产固定 [`Lifecycle::Legacy`]**。
//! - **T2.0（本模块下半部）**：四方法 [`McpMemoryClient::recall`] / [`remember`] /
//!   [`forget`] / [`consolidate`]，每个调用都带**独立超时**，失败一律降级为
//!   `Err(原因)` 交给调用方「跳过记忆继续」。
//!
//! [`remember`]: McpMemoryClient::remember
//! [`forget`]: McpMemoryClient::forget
//! [`consolidate`]: McpMemoryClient::consolidate
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
use std::time::Duration;

use anyhow::{Context, Result};
use rmcp::model::ProtocolVersion;
use rmcp::service::{ClientLifecycleMode, Peer, RoleClient, RunningService};
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

// ---------------------------------------------------------------------------
// T2.0 —— 四个业务方法
// ---------------------------------------------------------------------------

/// 单次 `recall` 的默认超时上限。
///
/// 必须有上限：`recall` 要走 embedding，`/upload` 是同步 HTTP，记忆层不能把它
/// 拖得更久 —— 宁可超时后跳过记忆（响应体带 `memory_skip_reason`）。
///
/// **取值来自真机标定**，不是拍的：Mac / 真库 5433 / nomic-embed-text(274MB)
/// 实测 recall 端到端 热 59–72ms、冷 59ms；embedding 模型冷加载单独测 0.65s
/// （`keep_alive:0` 卸载后重测，排除缓存假象）⇒ 最坏 ≈ 0.65s + SQL 10ms ≈ 0.7s。
/// 2s ≈ 3× 余量，是「本地模型」这一档的合理上界；不建议低于 1s。
pub const DEFAULT_RECALL_TIMEOUT: Duration = Duration::from_secs(2);

/// 单次写操作（`remember` / `forget` / `consolidate`）的默认超时上限。
/// 写比读宽松：写要算 embedding，且丢了要重来。
pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

/// 记忆层调用的结果。
///
/// `Err(String)` 是**给人看的跳过原因**，会被写进 `/upload` 响应体的
/// `memory_skip_reason`。按已拍定的失败语义：**跳过记忆继续**，不是失败退出 ——
/// 终端机的主链路是「优化简历」，记忆是增强项。
pub type MemoryResult<T> = std::result::Result<T, String>;

/// 一条检索结果。
///
/// cortex-mcp 的 `recall` 返回的是 JSON 数组，元素字段（`content` / `kind` /
/// `importance` / `created_at` …）由对端演进，这里刻意**不强类型化**：
/// 强类型会把还没定的契约提前焊死，对端加字段就得改一遍 Rust。
pub type MemoryHit = serde_json::Value;

/// 终端内嵌的记忆层客户端（cortex-mcp 作为 stdio 子进程）。
///
/// **连接是急切的**（[`connect`](Self::connect) 里完成握手），所以持有
/// `&self` 的四个方法都可以**并发**调用：调用走克隆出来的 [`Peer`]，
/// 不经过任何锁。子进程崩溃后的重启策略按计划后置：崩溃时调用返回 `Err`，
/// 调用方据此降级。
pub struct McpMemoryClient {
    /// 调用句柄。`Peer` 只含 channel + 元数据，克隆很便宜。
    peer: Peer<RoleClient>,
    /// 保活句柄 + 显式关闭入口。
    ///
    /// `RunningService::cancel` 需要 `self`，而 `Arc<McpMemoryClient>` 下拿不到
    /// `&mut` —— 所以把「调用」和「保活」拆开：调用用上面那个 `peer`，
    /// 关闭时把 service 从 Option 里 `take` 出来消费掉。
    /// `std::sync::Mutex` 只在 take 的瞬间持有，不跨 await。
    service: std::sync::Mutex<Option<RunningService<RoleClient, ()>>>,
    protocol_version: Option<String>,
    recall_timeout: Duration,
    write_timeout: Duration,
}

/// 从 `CallToolResult` 里取出「工具真正返回的东西」。
///
/// ## 为什么要这层（实测踩到的，不是预防性设计）
///
/// Python `mcp` 1.30 的 `FastMCP` 会把工具返回的**字符串**同时放进两处：
/// `content`（文本帧，原样）和 `structuredContent`（包成 `{"result": "..."}`）。
/// 若直接读 `structuredContent`，`recall` 拿到的是那个**包装对象**而不是记忆数组，
/// 于是「1 个包装」会被当成「1 条记忆」—— T2.0 冒烟第一次跑就是这样（`hits=1`）。
///
/// 所以顺序必须是：**文本帧优先** → 结构体兜底 → 结构体若是 `{"result": X}` 再解一层。
fn extract_payload(result: &rmcp::model::CallToolResult) -> Option<serde_json::Value> {
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    if !text.trim().is_empty() {
        return Some(serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)));
    }

    let structured = result.structured_content.clone()?;
    Some(unwrap_result_wrapper(structured))
}

/// 解开 FastMCP 的 `{"result": X}` 单键包装（只解一层）。
fn unwrap_result_wrapper(value: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(map) = &value {
        if map.len() == 1 {
            if let Some(inner) = map.get("result") {
                return inner.clone();
            }
        }
    }
    value
}

impl McpMemoryClient {
    /// 拉起子进程并握手。
    ///
    /// 生产一律传 [`Lifecycle::Legacy`] —— Mac 真机与 Dell 夹具对拍均确认：
    /// Python `mcp` 1.x 只认 `initialize`。
    pub async fn connect(spec: &McpServerSpec, lifecycle: Lifecycle) -> Result<Self> {
        let transport = TokioChildProcess::new(spec.to_command()).with_context(|| {
            format!(
                "无法拉起 MCP 子进程：{} {}（检查 program 路径与 args）",
                spec.program,
                spec.args.join(" ")
            )
        })?;
        let service = ()
            .serve_with_lifecycle(transport, lifecycle.to_mode())
            .await
            .with_context(|| {
                format!(
                    "MCP 握手失败（lifecycle={:?}）。常见原因：① Python 环境缺依赖；\
                     ② program 指向的不是能跑 cortex-mcp 的解释器；③ 对端 stdout 被日志污染。",
                    lifecycle
                )
            })?;
        Ok(Self {
            peer: service.peer().clone(),
            protocol_version: service
                .peer_info()
                .map(|i| i.protocol_version.as_str().to_string()),
            service: std::sync::Mutex::new(Some(service)),
            recall_timeout: DEFAULT_RECALL_TIMEOUT,
            write_timeout: DEFAULT_WRITE_TIMEOUT,
        })
    }

    /// 覆盖读/写超时（链式）。
    pub fn with_timeouts(mut self, recall: Duration, write: Duration) -> Self {
        self.recall_timeout = recall;
        self.write_timeout = write;
        self
    }

    /// 协商到的协议版本。
    pub fn protocol_version(&self) -> Option<String> {
        self.protocol_version.clone()
    }

    /// 子进程/传输是否仍然活着（轻量检查，不发起请求）。
    pub fn is_alive(&self) -> bool {
        !self.peer.is_transport_closed()
    }

    /// 服务端工具名清单（用于启动自检）。
    pub async fn list_tools(&self) -> Result<Vec<String>> {
        Ok(self
            .peer
            .list_all_tools()
            .await
            .context("tools/list 失败")?
            .into_iter()
            .map(|t| t.name.to_string())
            .collect())
    }

    /// 非消费式关闭（`Arc` 持有下也能调用；幂等）。
    ///
    /// 不关也不会泄漏：rmcp 的 `DropGuard` 在 `RunningService` 被 drop 时
    /// 会取消连接。这里提供显式入口，是为了让进程退出路径**可观测**。
    pub async fn close(&self) {
        let taken = {
            let mut guard = match self.service.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.take()
        };
        if let Some(service) = taken {
            let _ = service.cancel().await;
        }
    }

    /// 统一的工具调用：超时 + 错误提取 + 文本帧还原成 JSON。
    ///
    /// cortex-mcp 的工具全部返回**文本**，其中 `recall` / `consolidate_memories`
    /// 返回的是 JSON 字符串；这里尝试 `serde_json::from_str`，解析不了就原样
    /// 包成 `Value::String`（例如 `"no relevant memories"`）。
    async fn call_tool(
        &self,
        tool: &'static str,
        arguments: serde_json::Map<String, serde_json::Value>,
        timeout: Duration,
    ) -> MemoryResult<serde_json::Value> {
        let mut params = rmcp::model::CallToolRequestParams::default();
        params.name = std::borrow::Cow::Borrowed(tool);
        params.arguments = Some(arguments);

        let call = self.peer.call_tool(params);
        let result = match tokio::time::timeout(timeout, call).await {
            // ⏱️ 超时必须能中断等待：否则记忆层会把 /upload 一起拖死。
            Err(_) => {
                return Err(format!(
                    "MCP 工具 `{}` 超时（上限 {}ms）",
                    tool,
                    timeout.as_millis()
                ))
            }
            Ok(Err(e)) => return Err(format!("MCP 工具 `{}` 调用失败: {}", tool, e)),
            Ok(Ok(r)) => r,
        };

        if result.is_error == Some(true) {
            let detail = result
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!(
                "MCP 工具 `{}` 返回错误: {}",
                tool,
                if detail.is_empty() { "(无详情)" } else { &detail }
            ));
        }

        Ok(extract_payload(&result).unwrap_or(serde_json::Value::Null))
    }

    /// 检索记忆（cortex-mcp `recall`）。
    ///
    /// - `kind`：`working` / `episodic` / `semantic` / `procedural`，`None` 为不限；
    /// - `k`：返回条数。
    ///
    /// 对端在无命中时返回纯文本 `"no relevant memories"`，这里归一化成**空数组**，
    /// 免得调用方每次都要判断这是空结果还是错误。
    pub async fn recall(
        &self,
        query: &str,
        kind: Option<&str>,
        k: usize,
    ) -> MemoryResult<Vec<MemoryHit>> {
        let mut args = serde_json::Map::new();
        args.insert("query".into(), serde_json::Value::String(query.to_string()));
        args.insert("k".into(), serde_json::Value::from(k));
        if let Some(kind) = kind {
            args.insert("kind".into(), serde_json::Value::String(kind.to_string()));
        }

        let value = self.call_tool("recall", args, self.recall_timeout).await?;
        Ok(match value {
            serde_json::Value::Array(items) => items,
            serde_json::Value::Null => Vec::new(),
            serde_json::Value::String(s) if s.contains("no relevant memories") => Vec::new(),
            other => vec![other],
        })
    }

    /// 写入一条记忆（cortex-mcp `remember`）。
    ///
    /// `metadata` 按对端契约序列化成 **JSON 字符串**再传（对端签名是 `metadata: str`），
    /// 跨境合规保留键：`privacy_tier` / `source_region` / `dest_region` / `redacted`。
    ///
    /// ## 返回值语义（以真实 cortex-mcp 源码为准）
    ///
    /// 对端的 `remember` 有三种文案：
    /// - `"remembered <id> (<kind>)"` ⇒ `Ok(<id>)`（已抽出裸 id，可直接喂 `forget`）；
    /// - `"denied by cross-border policy: <verdict> …"` ⇒ `Err(拒绝原因)`
    ///   —— **合规拒绝绝不能当成功**，否则调用方会以为记忆已落库；
    /// - 其它 ⇒ `Ok(原文)`（保守透传，不猜）。
    pub async fn remember(
        &self,
        content: &str,
        kind: Option<&str>,
        metadata: Option<serde_json::Value>,
        importance: Option<f64>,
    ) -> MemoryResult<String> {
        let mut args = serde_json::Map::new();
        args.insert(
            "content".into(),
            serde_json::Value::String(content.to_string()),
        );
        if let Some(kind) = kind {
            args.insert("kind".into(), serde_json::Value::String(kind.to_string()));
        }
        if let Some(metadata) = metadata {
            let as_str = metadata.to_string();
            args.insert("metadata".into(), serde_json::Value::String(as_str));
        }
        if let Some(importance) = importance {
            args.insert("importance".into(), serde_json::Value::from(importance));
        }

        let value = self.call_tool("remember", args, self.write_timeout).await?;
        let raw = match value {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        };
        let raw = raw.trim();

        if let Some(rest) = raw.strip_prefix("remembered ") {
            // "remembered <id> (<kind>)" —— 第一个空白前就是裸 id
            let id = rest
                .split_whitespace()
                .next()
                .unwrap_or(rest)
                .trim()
                .to_string();
            return Ok(id);
        }
        if raw.starts_with("denied by cross-border policy") {
            return Err(format!("记忆写入被跨境策略拒绝: {}", raw));
        }
        Ok(raw.to_string())
    }

    /// 按 id 删除一条记忆（cortex-mcp `forget`）。
    ///
    /// 对端在删不到时返回 `"<id> not found"`（**不是错误**），所以这里用
    /// `Ok(false)` 表达「没删到」，只有调用失败才 `Err`。
    pub async fn forget(&self, memory_id: &str) -> MemoryResult<bool> {
        let mut args = serde_json::Map::new();
        args.insert(
            "memory_id".into(),
            serde_json::Value::String(memory_id.to_string()),
        );
        let value = self.call_tool("forget", args, self.write_timeout).await?;
        Ok(match &value {
            serde_json::Value::String(s) => !s.contains("not found"),
            _ => true,
        })
    }

    /// 元认知巩固（cortex-mcp `consolidate_memories`）。
    ///
    /// ⚠️ 工具名是 `consolidate_memories`，不是 `consolidate` —— 以真机
    /// cortex-mcp 的 `tools/list` 为准（T1.0 夹具当时写成 `consolidate`，
    /// 已同步修正，否则会 `tool not found`）。
    pub async fn consolidate(&self) -> MemoryResult<serde_json::Value> {
        self.call_tool("consolidate_memories", serde_json::Map::new(), self.write_timeout)
            .await
    }
}
