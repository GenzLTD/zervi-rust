//! # mcp-memory-seed —— A/B 注入阶段的记忆预置工具
//!
//! 为什么单独做一个二进制：`mcp-memory-smoke` 的 remember 内容是写死的测试串，
//! 不能拿来预置「同类岗位画像」。而注入阶段的 A/B 有一个硬前提 ——
//!
//! > on 组每一轮的记忆条数必须**恒定**，否则测到的是「1 条 vs 3 条」
//! > （`remember` 每次上传写 1 条），不是「有 / 无记忆」。
//!
//! 所以每轮 on 组请求前要先清库、再灌固定 K 条。清库用 psql 一行就够，
//! 灌库必须有能走 cortex-mcp（含 embedding）的工具 —— 就是本工具。
//!
//! ```text
//! # 每行一条，空行与 # 开头忽略
//! mcp-memory-seed --file preload.txt \
//!   --program /path/to/venv/bin/python --cwd /path/to/cortex-mcp \
//!   --env CORTEX_DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:5433/cortex
//! ```
//!
//! `--verify-query` 会在灌完后用**真实上传用的那条 query** 试 recall 一次，
//! 报出命中数与是否命中刚灌的 id。这一步防的是「灌进去了但 recall 查不到」——
//! 那种情况下注入块恒空，A/B 的 Δ 恒为 0，看起来像「记忆无效」，其实是白测。
//!
//! 退出码：0 = 全部写入成功；1 = 有失败（含 denied / 超时）；2 = 参数或文件问题。

use std::process::ExitCode;
use std::time::{Duration, Instant};

use mcp_client::{Lifecycle, McpMemoryClient, McpServerSpec};

struct Args {
    program: String,
    module: String,
    cwd: Option<String>,
    kind: String,
    importance: f64,
    file: Option<String>,
    texts: Vec<String>,
    env: Vec<(String, String)>,
    verify_query: Option<String>,
    dry_run: bool,
    write_timeout_ms: u64,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut a = Self {
            program: "python".into(),
            module: "cortex_mcp.server".into(),
            cwd: None,
            kind: "episodic".into(),
            importance: 0.6,
            file: None,
            texts: Vec::new(),
            env: Vec::new(),
            verify_query: None,
            dry_run: false,
            write_timeout_ms: mcp_client::DEFAULT_WRITE_TIMEOUT.as_millis() as u64,
        };
        let mut it = std::env::args().skip(1);
        while let Some(flag) = it.next() {
            let mut val = || it.next().ok_or_else(|| format!("{flag} 缺少取值"));
            match flag.as_str() {
                "--program" => a.program = val()?,
                "--module" => a.module = val()?,
                "--cwd" => a.cwd = Some(val()?),
                "--kind" => a.kind = val()?,
                "--importance" => {
                    a.importance = val()?.parse().map_err(|_| "--importance 需为小数")?
                }
                "--file" => a.file = Some(val()?),
                "--text" => a.texts.push(val()?),
                "--verify-query" => a.verify_query = Some(val()?),
                "--write-timeout-ms" => {
                    a.write_timeout_ms = val()?.parse().map_err(|_| "需为数字")?
                }
                "--dry-run" => a.dry_run = true,
                "--env" => {
                    let kv = val()?;
                    let (k, v) = kv
                        .split_once('=')
                        .ok_or_else(|| format!("--env 需要 KEY=VALUE，收到 {kv}"))?;
                    a.env.push((k.to_string(), v.to_string()));
                }
                "-h" | "--help" => {
                    println!("{}", HELP);
                    std::process::exit(0);
                }
                other => return Err(format!("未知参数 {other}（--help 看用法）")),
            }
        }
        Ok(a)
    }

    /// 收集待写入的记忆：文件行 + `--text`，空行与 `#` 注释跳过。
    fn collect(&self) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        if let Some(path) = &self.file {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("读不了 --file {path}：{e}"))?;
            for line in raw.lines() {
                let t = line.trim();
                if t.is_empty() || t.starts_with('#') {
                    continue;
                }
                out.push(t.to_string());
            }
        }
        out.extend(self.texts.iter().cloned());
        if out.is_empty() {
            return Err("没有任何待写入内容（用 --file 或 --text）".into());
        }
        Ok(out)
    }
}

const HELP: &str = "\
mcp-memory-seed —— A/B 注入前的记忆预置

  --file <path>            每行一条记忆（空行 / # 注释跳过）
  --text <字符串>          直接给一条，可重复
  --kind <kind>            默认 episodic
  --importance <小数>      默认 0.6
  --verify-query <query>   灌完用真实上传用的 query 试 recall，验证「灌了能查到」
  --dry-run                只打印不写入
  --write-timeout-ms <n>   单条写入超时，默认 15000
  --program <解释器>       默认 python（应指向装了 cortex-mcp 的 venv）
  --module <模块>          默认 cortex_mcp.server
  --cwd <cortex-mcp 根>    cortex-mcp 读 .env 与包路径需要
  --env KEY=VALUE          可重复，用于注入 CORTEX_DATABASE_URL 等";

#[tokio::main]
async fn main() -> ExitCode {
    let args = match Args::parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("参数错误：{e}");
            return ExitCode::from(2);
        }
    };

    let texts = match args.collect() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    println!("准备写入 {} 条记忆（kind={}）", texts.len(), args.kind);
    for (i, t) in texts.iter().enumerate() {
        // 只打前 40 字符：预置内容里可能有真实姓名/公司
        let head: String = t.chars().take(40).collect();
        println!("  [{}/{}] {}…", i + 1, texts.len(), head);
    }

    if args.dry_run {
        println!("--dry-run：未写入。");
        return ExitCode::SUCCESS;
    }

    let mut spec = McpServerSpec::python_module(args.program.clone(), args.module.clone());
    if let Some(cwd) = &args.cwd {
        spec = spec.with_cwd(cwd.clone());
    }
    for (k, v) in &args.env {
        spec = spec.with_env(k.clone(), v.clone());
    }

    let connect_started = Instant::now();
    println!(
        "连接 MCP 子进程（{}）：{} {}（cwd={}）",
        connect_started.elapsed().as_millis(),
        args.program,
        args.module,
        args.cwd.as_deref().unwrap_or("(继承)")
    );
    let client = match McpMemoryClient::connect(&spec, Lifecycle::Legacy).await {
        Ok(c) => {
            println!(
                "OK 已连接（protocolVersion={}，握手 {}ms）",
                c.protocol_version().as_deref().unwrap_or("(未知)"),
                connect_started.elapsed().as_millis()
            );
            c.with_timeouts(
                mcp_client::DEFAULT_RECALL_TIMEOUT,
                Duration::from_millis(args.write_timeout_ms),
            )
        }
        Err(e) => {
            eprintln!("FAIL 连接失败：{e:#}");
            return ExitCode::from(1);
        }
    };

    let mut ok_ids: Vec<String> = Vec::new();
    let mut failed = 0usize;
    for (i, text) in texts.iter().enumerate() {
        let started = Instant::now();
        match client
            .remember(
                text,
                Some(args.kind.as_str()),
                Some(serde_json::json!({"source": "mcp-memory-seed", "purpose": "ab-preload"})),
                Some(args.importance),
            )
            .await
        {
            Ok(reply) => {
                // 对端可能回整句「remembered <id> (<kind>)」，抽第一个 token 当 id
                let id = reply.split_whitespace().next().unwrap_or("(空)").to_string();
                println!(
                    "  OK  [{}/{}] id={} {}ms",
                    i + 1,
                    texts.len(),
                    id,
                    started.elapsed().as_millis()
                );
                ok_ids.push(id);
            }
            Err(e) => {
                failed += 1;
                eprintln!(
                    "  FAIL [{}/{}] {}ms：{:#}",
                    i + 1,
                    texts.len(),
                    started.elapsed().as_millis(),
                    e
                );
            }
        }
    }

    println!("写入完成：{} 成功 / {} 失败", ok_ids.len(), failed);

    // 关键验证：灌进去 ≠ recall 查得到。查不到时注入块恒空，A/B 会白跑。
    if let Some(query) = &args.verify_query {
        match client.recall(query, Some(args.kind.as_str()), 5).await {
            Ok(hits) => {
                // MemoryHit 是裸 JSON：cortex-mcp 的记录 id 在 `id` 键上；
                // 取不到就退化成整条记录的文本，至少能看出「命中的是不是刚灌的那批」。
                let hit_ids: Vec<String> = hits
                    .iter()
                    .map(|h| {
                        h.get("id")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| h.to_string())
                    })
                    .collect();
                let overlap = hit_ids.iter().filter(|id| ok_ids.iter().any(|o| o == *id)).count();
                println!(
                    "验证 recall：hits={}（query 前 40 字：{}…）",
                    hits.len(),
                    query.chars().take(40).collect::<String>()
                );
                println!("  命中本轮新灌的条数：{overlap}/{}", ok_ids.len());
                if hits.is_empty() || overlap == 0 {
                    println!(
                        "  ⚠️ 灌进去了但查不到 —— 此时注入块会恒空，A/B 的 Δ 恒为 0，\
                         看起来像「记忆无效」，其实是没测到。先改预置内容或 query。"
                    );
                }
            }
            Err(e) => {
                eprintln!("验证 recall 失败：{e:#}");
                failed += 1;
            }
        }
    }

    client.close().await;

    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
