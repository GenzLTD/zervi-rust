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
//! `--verify-query` 会在灌完后用**真实上传用的那条 query** 试 recall，
//! 把 top-k 的**原文**打出来，并用 `--require-topics` 做主题词闸门。
//!
//! ⚠️ 注意它能证明什么：cortex-mcp 的 `recall` **没有相似度阈值**
//! （向量 top-100 + 全文检索的排名融合，`ORDER BY rank DESC LIMIT k`），
//! 所以库非空时必然返回 k 条 —— 「命中几条」「命中本轮 K/K」都**不能**
//! 说明相关性，只说明写入成功、通道通。相关性只能看内容（本工具会打印），
//! 或用 `--require-topics` 断言。这一层不能省：注入阶段 `memory_injected`
//! 必定为 true，preload 内容跑偏时 Δ 无法归因到「记忆有用」。
//!
//! 退出码：
//! - `0` 全部写入成功，且所有闸门都过了（没配闸门 = 没有闸门要过）；
//! - `1` 有失败（含 denied / 超时 / recall 报错）；
//! - `2` 参数或文件问题（含「闸门配了却没配 `--verify-query`」——见下）；
//! - `3` **写得不对**：写入本身成功，但内容没通过闸门（缺主题词 / 出现禁用词）。
//!
//! 退出码 3 的单独立项不是洁癖：hook 里 `if seed; then run ab; fi` 只看得到
//! 成功/失败，分不清「没写进去」（1）和「写进去了但不是同类岗位」（3）。

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
    require_topics: Option<String>,
    forbid_strings: Option<String>,
    verify_only: bool,
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
            require_topics: None,
            forbid_strings: None,
            verify_only: false,
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
                "--require-topics" => a.require_topics = Some(val()?),
                "--forbid-strings" => a.forbid_strings = Some(val()?),
                "--verify-only" => a.verify_only = true,
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

/// 逗号分隔的闸门词表。
///
/// 返回 `Err` 表示**参数写错了**，与「内容不匹配」严格分开：`--require-topics ",,"`
/// 以前会让词表为空 ⇒ 匹配必然为空 ⇒ exit 3 +「preload 内容与本次岗位不匹配」，
/// 报的却是一条根本不存在的故障。参数错就该 exit 2。
fn parse_gate_list(raw: &str) -> Result<Vec<String>, String> {
    let terms: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    if terms.is_empty() {
        return Err(format!("闸门词表为空（收到 {raw:?}），至少给一个词"));
    }
    Ok(terms)
}

/// 从一条命中里取出可读文本。
///
/// cortex-mcp 现在返回 `content`，但返回形状不由我们控制（upload-service 侧的
/// `hit_text` 也是按常见键名逐个试）。闸门如果只认 `content`，键名一变就变成
/// 「一条都没匹配上」—— 又是一次静默失效。
fn hit_content(hit: &serde_json::Value) -> Option<&str> {
    if let Some(s) = hit.as_str() {
        return Some(s);
    }
    for key in ["content", "text", "memory", "summary", "body"] {
        if let Some(s) = hit.get(key).and_then(|v| v.as_str()) {
            return Some(s);
        }
    }
    None
}

/// 词表里哪些词在命中集里出现过（**大小写不敏感**：`rust` 不该因为写成 `Rust` 判失败）。
fn terms_present(hits: &[serde_json::Value], terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .filter(|t| {
            let needle = t.to_lowercase();
            hits.iter()
                .filter_map(hit_content)
                .any(|c| c.to_lowercase().contains(&needle))
        })
        .cloned()
        .collect()
}

const HELP: &str = "\
mcp-memory-seed —— A/B 注入前的记忆预置

  --file <path>            每行一条记忆（空行 / # 注释跳过）
  --text <字符串>          直接给一条，可重复
  --kind <kind>            默认 episodic
  --importance <小数>      默认 0.6
  --verify-query <query>   灌完用真实上传用的 query 试 recall，打印 top-k 原文
                           （cortex-mcp 无相似度阈值 ⇒ hits 数不代表相关性，必须看内容）
  --require-topics a,b,c   闸门：注入的 k 条里至少要有一条含其中某个主题词，
                           否则 exit 3（这条比「命中 K/K」诚实）
  --forbid-strings a,b,c   闸门：注入的 k 条里不许出现这些词（姓名/公司/学校），
                           出现即 exit 3（泄漏检查；脱敏只清证件/手机/邮箱）
  --verify-only            不新增记忆，只跑 --verify-query 审计现有库内容
                           （注意 recall 会更新 access_count/last_accessed）
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

    // 闸门必须和 --verify-query 一起出现。否则闸门整段被跳过、退出码 0，
    // 调用方的 hook 会当成「过了」—— 正是本轮在修的「看起来通过、其实没测」。
    if args.verify_query.is_none() {
        for (flag, given) in [
            ("--require-topics", args.require_topics.is_some()),
            ("--forbid-strings", args.forbid_strings.is_some()),
        ] {
            if given {
                eprintln!("{flag} 需要配 --verify-query：没有 query 就没有 recall，闸门无事可审");
                return ExitCode::from(2);
            }
        }
    }

    // 词表在**连接之前**解析完：参数错要当场 exit 2，不能等 recall 回来才报。
    let require_topics = match args.require_topics.as_deref().map(parse_gate_list) {
        None => None,
        Some(Ok(v)) => Some(v),
        Some(Err(e)) => {
            eprintln!("--require-topics：{e}");
            return ExitCode::from(2);
        }
    };
    let forbid_strings = match args.forbid_strings.as_deref().map(parse_gate_list) {
        None => None,
        Some(Ok(v)) => Some(v),
        Some(Err(e)) => {
            eprintln!("--forbid-strings：{e}");
            return ExitCode::from(2);
        }
    };

    // 审计模式：不新增记忆，只验。库里已有数据时用它，避免"为了检查又灌一遍"
    let texts = if args.verify_only {
        if args.verify_query.is_none() {
            eprintln!("--verify-only 需要配 --verify-query（否则无事可做）");
            return ExitCode::from(2);
        }
        Vec::new()
    } else {
        match args.collect() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        }
    };

    if !args.verify_only {
        println!("准备写入 {} 条记忆（kind={}）", texts.len(), args.kind);
        for (i, t) in texts.iter().enumerate() {
            // 只打前 40 字符：预置内容里可能有真实姓名/公司
            let head: String = t.chars().take(40).collect();
            println!("  [{}/{}] {}…", i + 1, texts.len(), head);
        }
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
    // 内容闸门未过：写入本身成功，但内容不合格（缺主题词 / 出现禁用词）⇒ 单独退出码
    let mut content_gate_failed = false;
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

    if args.verify_only {
        println!("--verify-only：未写入，直接审计现有库内容");
    } else {
        println!("写入完成：{} 成功 / {} 失败", ok_ids.len(), failed);
    }

    // 验证段。**先说清它能证明什么、不能证明什么**：
    //
    //   cortex-mcp 的 recall 是「向量 top-100 + 全文检索」的排名融合，但
    //   **没有任何相似度阈值**，最终 `ORDER BY rank DESC LIMIT k`。所以
    //   库非空时必然返回 k 条，与 query 相关与否无关。
    //
    //   ⇒ `hits > 0` 只证明「写入成功 + recall 通道通」；
    //   ⇒ 「命中本轮新灌 K/K」在非空库上是**恒真**，不能当相关性判据；
    //   ⇒ hits=0 基本只会在空库出现。
    //
    // 所以真正的判据是**看内容**：把注入的 top-k 原文打出来，人工（或
    // `--require-topics`）确认进 prompt 的是不是同类岗位。这一步不能省，
    // 因为注入阶段 `memory_injected` 必定为 true，内容跑偏时 Δ 无法归因。
    if let Some(query) = &args.verify_query {
        match client.recall(query, Some(args.kind.as_str()), 5).await {
            Ok(hits) => {
                println!(
                    "验证 recall：hits={}（query 前 40 字：{}…）",
                    hits.len(),
                    query.chars().take(40).collect::<String>()
                );
                println!("  进 prompt 的将是下面这些（按 rank 降序）：");
                for (i, h) in hits.iter().enumerate() {
                    let id = h
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(无 id)")
                        .to_string();
                    let content = h
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| h.to_string());
                    let head: String = content.chars().take(60).collect();
                    let fresh = if ok_ids.iter().any(|o| o == &id) {
                        " [本轮新灌]"
                    } else {
                        ""
                    };
                    println!("    {}. {} {}{}…", i + 1, id, head, fresh);
                }

                let overlap = hits
                    .iter()
                    .filter(|h| {
                        h.get("id")
                            .and_then(|v| v.as_str())
                            .is_some_and(|id| ok_ids.iter().any(|o| o == id))
                    })
                    .count();
                if args.verify_only {
                    // 审计模式下 ok_ids 恒空，打 "0/0" 只会误导（看着像一条都没命中）
                    println!("  本轮新灌：无（--verify-only 未写入，overlap 无意义）");
                } else {
                    println!(
                        "  命中本轮新灌：{overlap}/{}（⚠️ 此数在非空库上恒真，只证明写入+通道，不证明相关性）",
                        ok_ids.len()
                    );
                }

                if hits.is_empty() {
                    println!(
                        "  ⚠️ hits=0 —— 空库或全部被 kind 过滤。此时注入块恒空、Δ 恒为 0，\
                         看起来像「记忆无效」，其实是没测到。"
                    );
                }

                // 内容闸门。hits 为空时不跑：那是「没检索到」，上面已经报过，
                // 别重复告警，也别把「没测到」误判成「内容不对」(exit 3)。
                if !hits.is_empty() {
                    // ① 主题词：至少命中一个，否则算 preload 跑偏
                    if let Some(want) = &require_topics {
                        let matched = terms_present(&hits, want);
                        println!(
                            "  主题词命中：{}/{}（{}）",
                            matched.len(),
                            want.len(),
                            if matched.is_empty() {
                                "无".to_string()
                            } else {
                                matched.join("、")
                            }
                        );
                        if matched.is_empty() {
                            println!(
                                "  ⚠️ 注入的 {n} 条里没有任何一条含指定主题词 ⇒ preload 内容与本次\
                                 岗位不匹配。带着它跑 A/B，Δ≠0 也分不清是「记忆有用」还是「被无关\
                                 内容带偏」。先改 preload。",
                                n = hits.len()
                            );
                            content_gate_failed = true;
                        }
                    }

                    // ② 禁用词（姓名 / 公司 / 学校）：一个都不许出现。
                    // 为什么必须机械化：`redact()` 只清证件/手机/邮箱，姓名与公司是明确
                    // 交给 prompt 禁令兜的 —— 人眼在 5×60 字里找漏一个名字，比脚本容易漏。
                    if let Some(forbidden) = &forbid_strings {
                        let leaked = terms_present(&hits, forbidden);
                        if leaked.is_empty() {
                            println!("  禁用词：0/{} 未出现 ✓", forbidden.len());
                        } else {
                            println!(
                                "  ⚠️ 注入的 {} 条里出现了禁用词：{} ⇒ 这是泄漏：本轮要传的那份\
                                 简历的姓名/公司不该出现在参考段里。先改 preload。",
                                hits.len(),
                                leaked.join("、")
                            );
                            content_gate_failed = true;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("验证 recall 失败：{e:#}");
                failed += 1;
            }
        }
    }

    client.close().await;

    if failed > 0 {
        ExitCode::from(1)
    } else if content_gate_failed {
        // 写入成功但内容不合格：用一个分开的码，脚本 hook 能区分「没写进去」和「写得不对」
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 参数错 ≠ 内容不匹配：空词表必须 exit 2（以前是 exit 3 + 假故障）。
    #[test]
    fn gate_list_rejects_blank_terms() {
        assert!(parse_gate_list(",,").is_err());
        assert!(parse_gate_list("  ,  ").is_err());
        assert!(parse_gate_list("").is_err());
    }

    #[test]
    fn gate_list_trims_and_drops_blanks() {
        assert_eq!(
            parse_gate_list(" 后端 , 招聘,, Rust ").unwrap(),
            vec!["后端".to_string(), "招聘".to_string(), "Rust".to_string()]
        );
    }

    /// 闸门读不到 `content` 时不能静默判「不匹配」，要退回其它常见键。
    #[test]
    fn hit_content_falls_back_to_other_keys() {
        assert_eq!(hit_content(&json!({"content": "甲"})), Some("甲"));
        assert_eq!(hit_content(&json!({"text": "乙"})), Some("乙"));
        assert_eq!(hit_content(&json!("丙")), Some("丙"));
        assert_eq!(hit_content(&json!({"id": "只有 id"})), None);
    }

    /// 主题词：大小写不敏感；命中集里没有的键不能拖垮整条判定。
    #[test]
    fn terms_present_ignores_case_and_missing_content() {
        let hits = vec![
            json!({"id": "m-1", "content": "后端 岗位画像：Rust + MySQL"}),
            json!({"id": "m-2"}), // 无 content：不能因此 panic 或整体判负
        ];
        let terms = vec!["rust".to_string(), "前端".to_string()];
        assert_eq!(terms_present(&hits, &terms), vec!["rust".to_string()]);
    }

    /// 禁用词走同一套匹配：出现即非空 ⇒ 调用方 exit 3。
    #[test]
    fn terms_present_reports_leaked_strings() {
        let hits = vec![json!({"content": "曾在 中国建设银行 做后端"})];
        let forbidden = vec!["中国建设银行".to_string(), "张三".to_string()];
        assert_eq!(
            terms_present(&hits, &forbidden),
            vec!["中国建设银行".to_string()]
        );
    }
}
