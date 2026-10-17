//! # mcp-memory-smoke —— T2.0 四方法冒烟
//!
//! 与 `mcp-probe`（只验握手）互补：本二进制**真的调用四个工具**，
//! 验证 remember → recall → forget → consolidate 全链路，并打印每一步的
//! 耗时与结果，用来证明「recall 有超时上限、失败可降级」这两条。
//!
//! ```text
//! mcp-memory-smoke --program <python> --module cortex_mcp.server --cwd <cortex-mcp 根>
//! mcp-memory-smoke --program <python> --module probe_server --cwd <fixtures 目录>
//! ```
//!
//! 退出码：0 = 四步全过；1 = 有失败（但仍会打印已完成的部分）。

use std::process::ExitCode;
use std::time::{Duration, Instant};

use mcp_client::{Lifecycle, McpMemoryClient, McpServerSpec};

struct Args {
    program: String,
    module: String,
    cwd: Option<String>,
    recall_timeout_ms: u64,
    write_timeout_ms: u64,
    env: Vec<(String, String)>,
}

impl Args {
    fn parse() -> Self {
        let mut a = Self {
            program: "python".into(),
            module: "cortex_mcp.server".into(),
            cwd: None,
            recall_timeout_ms: mcp_client::DEFAULT_RECALL_TIMEOUT.as_millis() as u64,
            write_timeout_ms: mcp_client::DEFAULT_WRITE_TIMEOUT.as_millis() as u64,
            env: Vec::new(),
        };
        let mut it = std::env::args().skip(1);
        while let Some(flag) = it.next() {
            let mut val = || it.next().unwrap_or_else(|| panic!("{flag} 缺少取值"));
            match flag.as_str() {
                "--program" => a.program = val(),
                "--module" => a.module = val(),
                "--cwd" => a.cwd = Some(val()),
                "--recall-timeout-ms" => a.recall_timeout_ms = val().parse().expect("需为数字"),
                "--write-timeout-ms" => a.write_timeout_ms = val().parse().expect("需为数字"),
                // KEY=VALUE，可重复；用于注入 CORTEX_DATABASE_URL 等
                "--env" => {
                    let kv = val();
                    let (k, v) = kv
                        .split_once('=')
                        .unwrap_or_else(|| panic!("--env 需要 KEY=VALUE，收到 {kv}"));
                    a.env.push((k.to_string(), v.to_string()));
                }
                other => panic!("未知参数 {other}"),
            }
        }
        a
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    let mut spec = McpServerSpec::python_module(args.program.clone(), args.module.clone());
    if let Some(cwd) = &args.cwd {
        spec = spec.with_cwd(cwd.clone());
    }
    for (k, v) in &args.env {
        spec = spec.with_env(k.clone(), v.clone());
    }

    println!(
        "spec: {} -m {} (cwd={})",
        args.program,
        args.module,
        args.cwd.as_deref().unwrap_or("(继承)")
    );

    let t0 = Instant::now();
    let client = match McpMemoryClient::connect(&spec, Lifecycle::Legacy).await {
        Ok(c) => c,
        Err(e) => {
            println!("[FAIL] connect      {}ms  {e:#}", t0.elapsed().as_millis());
            return ExitCode::from(1);
        }
    };
    let client = client.with_timeouts(
        Duration::from_millis(args.recall_timeout_ms),
        Duration::from_millis(args.write_timeout_ms),
    );
    println!(
        "[PASS] connect      {}ms  protocolVersion={}",
        t0.elapsed().as_millis(),
        client.protocol_version().unwrap_or_else(|| "(未协商)".into())
    );

    let mut all_ok = true;

    // 工具清单：四个业务方法依赖真实工具名（注意第 4 个是 consolidate_memories）
    let t = Instant::now();
    match client.list_tools().await {
        Ok(tools) => {
            let need = [
                "remember",
                "recall",
                "forget",
                "consolidate_memories",
            ];
            let missing = need
                .iter()
                .filter(|n| !tools.iter().any(|t| t == *n))
                .collect::<Vec<_>>();
            let ok = missing.is_empty();
            all_ok &= ok;
            println!(
                "[{}] tools/list   {}ms  {:?}{}",
                if ok { "PASS" } else { "FAIL" },
                t.elapsed().as_millis(),
                tools,
                if ok {
                    String::new()
                } else {
                    format!("  缺失: {:?}", missing)
                }
            );
        }
        Err(e) => {
            all_ok = false;
            println!("[FAIL] tools/list   {}ms  {e}", t.elapsed().as_millis());
        }
    }

    // ① remember
    let marker = format!(
        "mcp-memory-smoke 探针记录 {}",
        epoch_marker()
    );
    let t = Instant::now();
    let memory_id = match client
        .remember(
            &marker,
            Some("episodic"),
            Some(serde_json::json!({
                "source": "zervi-rust/mcp-memory-smoke",
                "privacy_tier": "public",
            })),
            Some(0.5),
        )
        .await
    {
        Ok(id) => {
            println!("[PASS] remember     {}ms  -> {id}", t.elapsed().as_millis());
            Some(id)
        }
        Err(e) => {
            all_ok = false;
            println!("[FAIL] remember     {}ms  {e}", t.elapsed().as_millis());
            None
        }
    };

    // ② recall（带超时上限）
    let t = Instant::now();
    match client.recall(&marker, Some("episodic"), 5).await {
        Ok(hits) => {
            // 注意：真实 cortex-mcp 的 recall 会走向量检索，命中条数取决于 embedding，
            // 这里不把「必须命中」当断言，只断言调用在超时内返回。
            //
            // 但**首条命中的形状**要看一眼：如果拿回来的是 FastMCP 的 {"result": ...}
            // 包装，说明 extract_payload 的取值顺序错了（T2.0 冒烟第一次就是这样）。
            let first = hits
                .first()
                .map(|h| {
                    h.get("content")
                        .and_then(|c| c.as_str())
                        .map(|s| truncate(s, 60))
                        .unwrap_or_else(|| truncate(&h.to_string(), 60))
                })
                .unwrap_or_else(|| "—".to_string());
            let ok = !first.contains("\"result\"");
            all_ok &= ok;
            println!(
                "[{}] recall       {}ms  hits={}（上限 {}ms）首条={}",
                if ok { "PASS" } else { "FAIL" },
                t.elapsed().as_millis(),
                hits.len(),
                args.recall_timeout_ms,
                first
            );
        }
        Err(e) => {
            let timed_out = e.contains("超时");
            // 超时是**设计内行为**（降级），不是缺陷：标 DEGRADED 但仍算通路通过。
            println!(
                "[{}] recall       {}ms  {e}",
                if timed_out { "DEGRADED" } else { "FAIL" },
                t.elapsed().as_millis()
            );
            if !timed_out {
                all_ok = false;
            }
        }
    }

    // ③ forget（有 id 才删）
    if let Some(id) = memory_id {
        let t = Instant::now();
        match client.forget(&id).await {
            Ok(true) => println!("[PASS] forget       {}ms  removed", t.elapsed().as_millis()),
            Ok(false) => println!(
                "[PASS] forget       {}ms  not found（对端语义，非错误）",
                t.elapsed().as_millis()
            ),
            Err(e) => {
                all_ok = false;
                println!("[FAIL] forget       {}ms  {e}", t.elapsed().as_millis());
            }
        }
    }

    // ④ consolidate_memories
    let t = Instant::now();
    match client.consolidate().await {
        Ok(v) => println!(
            "[PASS] consolidate  {}ms  {}",
            t.elapsed().as_millis(),
            truncate(&v.to_string(), 160)
        ),
        Err(e) => {
            all_ok = false;
            println!("[FAIL] consolidate  {}ms  {e}", t.elapsed().as_millis());
        }
    }

    client.close().await;
    println!("alive after close: {}", client.is_alive());

    if all_ok {
        println!("\n结果：四方法全通");
        ExitCode::SUCCESS
    } else {
        println!("\n结果：有失败项（见上方 FAIL）");
        ExitCode::from(1)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut = s.chars().take(n).collect::<String>();
    format!("{cut}…")
}

/// 不引 chrono（本 crate 不需要额外依赖），用 SystemTime 拼一个可读时间戳。
fn epoch_marker() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch-{secs}")
}
