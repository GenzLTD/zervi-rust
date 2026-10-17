//! T1.0 握手探针 —— 只做 `initialize` + `tools/list`，不调任何业务工具。
//!
//! 与 Mac 侧 `/tmp/cortex-verify/probe_mcp.py`（Python 最小 client）对拍用：
//! 同一个 cortex-mcp 子进程、同一个 lifecycle，两套 SDK 各自报出的
//! `protocolVersion` / 工具清单应当一致。
//!
//! ```text
//! # 最稳的一条（与 Python mcp 1.x 对齐）
//! cargo run -p mcp-client --bin mcp-probe -- --program python3
//!
//! # 确认对端到底支不支持新 spec 的 server/discover
//! cargo run -p mcp-client --bin mcp-probe -- --program python3 --lifecycle auto
//!
//! # 带 cortex-mcp 的数据库 DSN
//! cargo run -p mcp-client --bin mcp-probe -- --program python3 \
//!     --env CORTEX_PG_DSN=postgresql://localhost:5433/cortex
//! ```

use std::process::ExitCode;

use mcp_client::{probe_handshake, Lifecycle, McpServerSpec};

fn usage() -> &'static str {
    "\
T1.0 MCP 握手探针

用法:
  mcp-probe [选项]

选项:
  --program <PATH>      Python 解释器（默认: 依次试 python3, python）
  --module <NAME>       MCP 服务模块（默认: cortex_mcp.server）
  --cwd <DIR>           子进程工作目录（cortex-mcp 读 wukong-core/rules 时需要）
  --env <K=V>           追加环境变量，可重复（如 CORTEX_PG_DSN=...）
  --lifecycle <MODE>    legacy | auto | discover（默认: legacy）
  --json                只输出 JSON（便于对拍 / 落盘）
  -h, --help            显示本帮助

退出码: 0 成功 / 2 失败"
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut program: Option<String> = None;
    let mut module = "cortex_mcp.server".to_string();
    let mut cwd: Option<String> = None;
    let mut env: Vec<(String, String)> = Vec::new();
    let mut lifecycle = Lifecycle::Legacy;
    let mut json_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "--program" => match args.next() {
                Some(v) => program = Some(v),
                None => return fail("--program 需要一个值"),
            },
            "--module" => match args.next() {
                Some(v) => module = v,
                None => return fail("--module 需要一个值"),
            },
            "--cwd" => match args.next() {
                Some(v) => cwd = Some(v),
                None => return fail("--cwd 需要一个值"),
            },
            "--env" => match args.next() {
                Some(kv) => match kv.split_once('=') {
                    Some((k, v)) => env.push((k.to_string(), v.to_string())),
                    None => return fail(&format!("--env 需要 K=V 形式，收到: {kv}")),
                },
                None => return fail("--env 需要一个 K=V"),
            },
            "--lifecycle" => match args.next() {
                Some(v) => match Lifecycle::parse(&v) {
                    Some(l) => lifecycle = l,
                    None => return fail(&format!("未知 lifecycle: {v}（可选 legacy/auto/discover）")),
                },
                None => return fail("--lifecycle 需要一个值"),
            },
            "--json" => json_only = true,
            other => return fail(&format!("未知参数: {other}（用 --help 看用法）")),
        }
    }

    // 没指定 program 时按顺序探测，取第一个能拉起的。Windows 上是 python.exe。
    let candidates: Vec<String> = match program {
        Some(p) => vec![p],
        None => vec!["python3".into(), "python".into()],
    };

    let mut last_err: Option<String> = None;
    for prog in &candidates {
        let mut spec = McpServerSpec::python_module(prog.clone(), module.clone());
        if let Some(dir) = &cwd {
            spec = spec.with_cwd(dir);
        }
        for (k, v) in &env {
            spec = spec.with_env(k.clone(), v.clone());
        }

        if !json_only {
            eprintln!("[T1.0] 尝试拉起: {} -m {}", prog, module);
        }

        match probe_handshake(&spec, lifecycle).await {
            Ok(report) => {
                if json_only {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": true,
                            "program": prog,
                            "module": module,
                            "lifecycle": format!("{:?}", report.lifecycle),
                            "protocolVersion": report.protocol_version,
                            "serverInfo": {
                                "name": report.server_name,
                                "version": report.server_version,
                            },
                            "tools": report.tools,
                        })
                    );
                } else {
                    println!();
                    println!("=== T1.0 握手结果 ===");
                    println!("program          : {prog} -m {module}");
                    println!("lifecycle        : {:?}", report.lifecycle);
                    println!("protocolVersion  : {}", report.protocol_version);
                    println!("serverInfo.name  : {}", report.server_name);
                    println!("serverInfo.ver   : {}", report.server_version);
                    println!("tools ({})       : {}", report.tools.len(), report.tools.join(", "));
                    println!();
                    println!("--- JSON ---");
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": true,
                            "program": prog,
                            "module": module,
                            "lifecycle": format!("{:?}", report.lifecycle),
                            "protocolVersion": report.protocol_version,
                            "serverInfo": {
                                "name": report.server_name,
                                "version": report.server_version,
                            },
                            "tools": report.tools,
                        })
                    );
                }
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                // anyhow 的 {:#} 会把 context 链摊平，比 {:?} 更适合给人看。
                last_err = Some(format!("{e:#}"));
                if !json_only {
                    eprintln!("[T1.0] {} 失败: {:#}", prog, e);
                }
            }
        }
    }

    let msg = last_err.unwrap_or_else(|| "没有可尝试的 Python 解释器".to_string());
    if json_only {
        println!(
            "{}",
            serde_json::json!({ "ok": false, "lifecycle": format!("{lifecycle:?}"), "error": msg })
        );
    } else {
        eprintln!();
        eprintln!("[T1.0] 全部尝试失败。最后一次错误:");
        eprintln!("{msg}");
        eprintln!();
        eprintln!("排查顺序建议:");
        eprintln!("  1) 子进程起不来 → 先手工跑 `python -m cortex_mcp.server`，看缺哪个依赖");
        eprintln!("  2) 起得来但握手失败 → 换 --lifecycle auto 再试，确认是否 discover 不被识别");
        eprintln!("  3) stdout 被日志污染 → cortex-mcp 启动横幅必须写到 stderr，不能写 stdout");
    }
    ExitCode::from(2)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("参数错误: {msg}");
    eprintln!();
    eprintln!("{}", usage());
    ExitCode::from(2)
}
