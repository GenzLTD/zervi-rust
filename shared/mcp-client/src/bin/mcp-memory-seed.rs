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
//! - `1` **没写进去 / 调用本身坏了**：remember 失败（denied / 传输错），或**校验阶段
//!   recall 报错**（服务端 is_error / 非法 kind / 传输失败）。这些意味着「命令或连接有
//!   问题」，hook 应去修调用而不是重灌数据；
//! - `2` 参数或文件问题（含「闸门配了却没配 `--verify-query`」——见下）；
//! - `3` **写进去了但这批注入不能用**：内容没通过闸门（缺主题词 / 出现禁用词）、
//!   或**校验超时**（本地嵌入冷启动，见 [`DEFAULT_VERIFY_TIMEOUT_MS`]，超时只说明
//!   「这次没验成」不是「没写进去」）、或**无从判定** —— `hits=0`（空库 / kind 全被
//!   过滤，注入块恒空、Δ 恒 0），或 hits 非空但一条可读文本都取不到（返回形状变了，
//!   闸门读不到内容就只会判「不匹配/未泄漏」，那是假通过）。
//!
//! 校验阶段的 recall 失败分两码：含「超时」⇒ 3（运行期，非写入问题）；其余 recall 报错
//! ⇒ 1（调用坏了，含非法 kind —— 用户明确要求非法 kind 复跑期望 exit 1）。这条线把
//! 「没写进去」（1）和「写进去了但没法用 / 没验成」（3）彻底分开，hook 才分得清
//! 「重灌」和「加超时 / 修调用」。
//!
//! `hits=0` 与「读不出文本」这两种以前是 **exit 0**：闸门一声不响跳过，hook 当成
//! 「过了」—— 正是本工具反复栽的「看起来通过、其实没测」。
//!
//! 退出码 3 的单独立项不是洁癖：hook 里 `if seed; then run ab; fi` 只看得到
//! 成功/失败，分不清「没写进去」（1）和「写进去了但不能用」（3）。

use std::process::ExitCode;
use std::time::{Duration, Instant};

use mcp_client::{Lifecycle, McpMemoryClient, McpServerSpec};

/// 校验用的 recall 超时默认值。
///
/// **不能**复用 `DEFAULT_RECALL_TIMEOUT`（2s）：那是上传路径的预算，为的是记忆层
/// 拖不死 `/upload`。校验场景不一样 —— 第一次 recall 要冷启动本地嵌入模型：
/// Mac 实测空库首次直接超时（被误判成「没写进去」exit 1），热身后才 0.305s。
/// 30s 盖得住一次本地嵌入冷启动，又不至于让流程卡死。
const DEFAULT_VERIFY_TIMEOUT_MS: u64 = 30_000;

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
    verify_timeout_ms: u64,
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
            verify_timeout_ms: DEFAULT_VERIFY_TIMEOUT_MS,
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
                "--verify-timeout-ms" => {
                    a.verify_timeout_ms = val()?.parse().map_err(|_| "需为数字")?
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

/// 闸门判定结果。打印需要的明细都在这里，退出码只看 `verdict`。
struct GateReport {
    /// 命中条数（recall 返回的 k）。
    hits: usize,
    /// 命中里**能读出文本**的条数。为 0 而 hits 非空 ⇒ 返回形状变了，闸门读不到东西。
    readable: usize,
    /// 主题词里在命中集出现过的（大小写不敏感）。
    matched_topics: Vec<String>,
    /// 禁用词里在命中集出现过的 —— 非空即泄漏。
    leaked: Vec<String>,
    /// `None` = 通过；`Some(_)` = 一律 exit 3。
    verdict: Option<GateVerdict>,
}

/// 为什么判不过。**全部映射到 exit 3**，与「没写进去」(1) / 「参数错」(2) 分开。
#[derive(Debug, PartialEq)]
enum GateVerdict {
    /// 一条都没检索到：空库或 kind 全被过滤 ⇒ 注入块恒空、Δ 恒 0，等于没测。
    EmptyHits,
    /// 有 hits 但一条可读文本都取不到：闸门只会判「不匹配 / 未泄漏」，是假通过。
    Unreadable,
    /// 主题词一个都没命中：preload 内容与本次岗位不匹配。
    TopicsMismatch,
    /// 出现禁用词：本轮简历的姓名 / 公司漏进了参考段。
    Leaked,
}

/// 把「能不能用这批注入跑 A/B」压成一个判定。
///
/// 单独抽出来是为了能单测：这段以前内联在 `main` 里，`hits=0` 只 println 一句
/// 警告、退出码仍是 0 —— 谁也没法给它写断言。判定顺序按危害从大到小：
/// 先看有没有东西可判，再看内容对不对，最后才看泄漏。
fn evaluate_gates(
    hits: &[serde_json::Value],
    topics: Option<&[String]>,
    forbidden: Option<&[String]>,
) -> GateReport {
    let readable = hits.iter().filter(|h| hit_content(h).is_some()).count();
    let matched_topics = topics.map(|t| terms_present(hits, t)).unwrap_or_default();
    let leaked = forbidden
        .map(|f| terms_present(hits, f))
        .unwrap_or_default();

    let verdict = if hits.is_empty() {
        Some(GateVerdict::EmptyHits)
    } else if readable == 0 {
        Some(GateVerdict::Unreadable)
    } else if topics.is_some() && matched_topics.is_empty() {
        Some(GateVerdict::TopicsMismatch)
    } else if !leaked.is_empty() {
        Some(GateVerdict::Leaked)
    } else {
        None
    };

    GateReport {
        hits: hits.len(),
        readable,
        matched_topics,
        leaked,
        verdict,
    }
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
                           两个闸门都注意：hits=0、或命中集里一条可读文本都取不到时
                           **也是 exit 3**（无从判定，不能算通过）
  --verify-only            不新增记忆，只跑 --verify-query 审计现有库内容
                           （注意 recall 会更新 access_count/last_accessed）
  --dry-run                只打印不写入
  --write-timeout-ms <n>   单条写入超时，默认 15000
  --verify-timeout-ms <n>  校验 recall 超时，默认 30000
                           别调成 2000：那会让空库在本地嵌入冷启动时超时，
                           误判成「没写进去」(exit 1)。上传路径的 2s 不适用于这里
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
                Duration::from_millis(args.verify_timeout_ms),
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
    // 闸门未过：写入本身成功，但这批注入不能拿来跑 A/B —— 内容不合格（缺主题词 /
    // 出现禁用词），或**无从判定**（hits=0 / 一条可读文本都取不到）⇒ 单独退出码 3
    let mut gate_failed = false;
    for (i, text) in texts.iter().enumerate() {
        let started = Instant::now();
        match client
            .remember(
                text,
                Some(args.kind.as_str()),
                Some(serde_json::json!({"source": "mcp-memory-seed", "purpose": "ab-preload"})),
                Some(args.importance),
                Some("participant"),
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
        match client.recall(query, Some(args.kind.as_str()), 5, Some("observer")).await {
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
                    // 与闸门同一口径（`hit_content`）：打印的必须是闸门实际读的那段文本。
                    // 否则键名一变，人眼看到的（整段 JSON）和判定的（读不到）不是同一样东西。
                    let content = hit_content(h)
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

                let gates =
                    evaluate_gates(&hits, require_topics.as_deref(), forbid_strings.as_deref());

                // ① 无从判定：没检索到 / 读不出文本。这两种以前只打 ⚠️ 就 exit 0，
                // hook 当成「过了」—— 「没测到」不能让 A/B 继续跑，归 exit 3。
                if let Some(reason) = &gates.verdict {
                    gate_failed = true;
                    match reason {
                        GateVerdict::EmptyHits => println!(
                            "  ⚠️ hits=0 —— 空库或全部被 kind 过滤。此时注入块恒空、Δ 恒为 0，\
                             看起来像「记忆无效」，其实是没测到 ⇒ exit 3（不是 0）"
                        ),
                        GateVerdict::Unreadable => println!(
                            "  ⚠️ hits={} 但一条可读文本都取不到（没有 content/text/memory/… 键）\
                             ⇒ 闸门读不到内容，只会判「不匹配/未泄漏」，那是假通过 ⇒ exit 3",
                            gates.hits
                        ),
                        // 内容层面的失败在下面按闸门逐条打（那里有明细可展示）
                        GateVerdict::TopicsMismatch | GateVerdict::Leaked => {}
                    }
                }

                // ② 内容判定。只有读得出文本才谈得上；读不到时上面已报 Unreadable，
                // 别在这里再判一次「不匹配」——那是同一件事说两遍，还会误导成内容问题。
                if gates.readable > 0 {
                    // 主题词：至少命中一个，否则算 preload 跑偏
                    if let Some(want) = &require_topics {
                        println!(
                            "  主题词命中：{}/{}（{}）",
                            gates.matched_topics.len(),
                            want.len(),
                            if gates.matched_topics.is_empty() {
                                "无".to_string()
                            } else {
                                gates.matched_topics.join("、")
                            }
                        );
                        if gates.matched_topics.is_empty() {
                            println!(
                                "  ⚠️ 注入的 {n} 条里没有任何一条含指定主题词 ⇒ preload 内容与本次\
                                 岗位不匹配。带着它跑 A/B，Δ≠0 也分不清是「记忆有用」还是「被无关\
                                 内容带偏」。先改 preload。",
                                n = gates.hits
                            );
                        }
                    }

                    // 禁用词（姓名 / 公司 / 学校）：一个都不许出现。
                    // 为什么必须机械化：`redact()` 只清证件/手机/邮箱，姓名与公司是明确
                    // 交给 prompt 禁令兜的 —— 人眼在 5×60 字里找漏一个名字，比脚本容易漏。
                    if let Some(forbidden) = &forbid_strings {
                        if gates.leaked.is_empty() {
                            println!("  禁用词：0/{} 未出现 ✓", forbidden.len());
                        } else {
                            println!(
                                "  ⚠️ 注入的 {} 条里出现了禁用词：{} ⇒ 这是泄漏：本轮要传的那份\
                                 简历的姓名/公司不该出现在参考段里。先改 preload。",
                                gates.hits,
                                gates.leaked.join("、")
                            );
                        }
                    }
                }
            }
            Err(e) => {
                // 两类失败要分开，因为它们指向完全不同的责任方：
                //
                // - **超时**（错误文案含「超时」）：校验阶段的运行期超时，通常是本地
                //   嵌入冷启动尚未就绪。它不是「没写进去」—— 归 exit 3。否则 hook 看到
                //   1 会去重灌 preload，而真实原因是「这次没验成，加超时/先热身」。
                //   （2s 默认已改 30s 盖住一次冷启动，这是兜底。）
                //
                // - **recall 报错 / 服务端 is_error / 传输失败**（非法 kind 等）：调用本身
                //   坏了，归 exit 1。用户明确要求非法 kind 复跑期望 exit 1
                //   （「recall 返回非命中文案，exit 1」）—— 那是「你的命令/连接有问题」，
                //   hook 该去修调用，不是重灌数据。
                //
                // 区分靠文案：客户端超时固定输出「MCP 工具 `recall` 超时（上限 Nms）」，
                // 其余失败是「返回错误 / 调用失败」。
                if e.contains("超时") {
                    eprintln!("验证 recall 超时：{e}");
                    eprintln!("  ⇒ exit 3（不是 1）：校验超时，不是没写进去 —— 先热身/加 --verify-timeout-ms");
                    gate_failed = true;
                } else {
                    eprintln!("验证 recall 失败：{e}");
                    eprintln!("  => exit 1: recall 报错/is_error(含非法 kind)，调用本身有问题");
                    failed += 1;
                }
            }
        }
    }

    client.close().await;

    if failed > 0 {
        // 「没写进去」比「写得不对」更根本，优先归 1
        ExitCode::from(1)
    } else if gate_failed {
        // 写进去了但这批注入不能用：内容不合格，或压根没验到（hits=0 / 读不出文本）。
        // 用一个分开的码，脚本 hook 能区分「没写进去」和「写得不对/没测到」。
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

    /// hits=0 = 没检索到，注入块恒空、Δ 恒 0 ⇒ 不能算通过。
    /// 以前这里只打一句警告、退出码 0，hook 当成「过了」（P0）。
    /// 注意：**没配任何闸门也一样判失败** —— 没内容就没验证发生。
    #[test]
    fn empty_hits_are_unverifiable() {
        let gates = evaluate_gates(&[], None, None);
        assert_eq!(gates.verdict, Some(GateVerdict::EmptyHits));
        assert_eq!(gates.readable, 0);
    }

    /// 有 hits 但一条可读文本都取不到（返回形状变了）⇒ 闸门只会判「不匹配/未泄漏」，
    /// 那是假通过（P1）。以前 `--forbid-strings` 会打印「0/N 未出现 ✓」并 exit 0。
    #[test]
    fn unreadable_hits_fail_instead_of_passing() {
        let hits = vec![json!({"id": "m-1", "metadata": {"kind": "episodic"}})];
        let forbidden = vec!["张三".to_string()];

        let with_gate = evaluate_gates(&hits, None, Some(&forbidden));
        assert_eq!(with_gate.hits, 1);
        assert_eq!(with_gate.readable, 0);
        assert_eq!(with_gate.verdict, Some(GateVerdict::Unreadable));
        // 关键：禁用词确实一个都没匹配上，但那不代表「没泄漏」，只代表「读不到」
        assert!(with_gate.leaked.is_empty());

        // 没配闸门也一样：读不出内容就无从判定
        assert_eq!(
            evaluate_gates(&hits, None, None).verdict,
            Some(GateVerdict::Unreadable)
        );
    }

    /// 正常路径：内容可读、主题词命中、无禁用词 ⇒ 通过。
    #[test]
    fn readable_hits_with_topics_pass() {
        let hits = vec![
            json!({"id": "m-1", "content": "后端 岗位画像：Rust + MySQL"}),
            json!({"id": "m-2", "content": "招聘 后端 工程师，要求 3 年"}),
        ];
        let topics = vec!["后端".to_string(), "Rust".to_string()];
        let forbidden = vec!["张三".to_string()];

        let gates = evaluate_gates(&hits, Some(&topics), Some(&forbidden));
        assert_eq!(gates.readable, 2);
        assert_eq!(
            gates.matched_topics,
            vec!["后端".to_string(), "Rust".to_string()]
        );
        assert!(gates.leaked.is_empty());
        assert_eq!(gates.verdict, None);
    }

    /// 主题词一个都没命中 ⇒ 不是「通过」，是 preload 跑偏。
    #[test]
    fn topic_mismatch_is_failure() {
        let hits = vec![json!({"content": "游戏策划 岗位画像"})];
        let topics = vec!["后端".to_string()];
        assert_eq!(
            evaluate_gates(&hits, Some(&topics), None).verdict,
            Some(GateVerdict::TopicsMismatch)
        );
    }

    /// 泄漏优先级高于通过：哪怕主题词全中、内容很像，出现禁用词就是不合格。
    #[test]
    fn leak_fails_even_when_topics_match() {
        let hits = vec![json!({"content": "后端 Rust，曾在 中国建设银行"})];
        let topics = vec!["后端".to_string()];
        let forbidden = vec!["中国建设银行".to_string()];

        let gates = evaluate_gates(&hits, Some(&topics), Some(&forbidden));
        assert_eq!(gates.verdict, Some(GateVerdict::Leaked));
        assert_eq!(gates.leaked, vec!["中国建设银行".to_string()]);
    }

    /// 没配闸门时，内容可读就该 exit 0 —— 别把「只想看一眼库里有啥」也判失败。
    #[test]
    fn readable_hits_without_gates_pass() {
        let hits = vec![json!({"content": "任意内容"})];
        assert_eq!(evaluate_gates(&hits, None, None).verdict, None);
    }

    /// 校验超时默认 30s，**不能**退回上传路径的 2s。
    /// 校验首次 recall 要冷启动本地嵌入（Mac 实测空库首次直接超时，被误判成
    /// 「没写进去」exit 1）；30s 盖得住一次冷启动。这条锁住「别手滑改回 2000」。
    #[test]
    fn verify_timeout_default_is_thirty_seconds() {
        assert_eq!(DEFAULT_VERIFY_TIMEOUT_MS, 30_000);
        assert_ne!(
            DEFAULT_VERIFY_TIMEOUT_MS,
            mcp_client::DEFAULT_RECALL_TIMEOUT.as_millis() as u64
        );
    }
}
