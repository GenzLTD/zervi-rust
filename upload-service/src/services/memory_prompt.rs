//! 记忆 → 提取提示词的注入层。
//!
//! 记忆命中本身是 `serde_json::Value`（cortex-mcp 的返回形状不由我们控制），
//! 而且内容是**上一批文件的摘要**。直接拼进 prompt 有三个真实故障：
//!
//! 1. **串号**：上一份简历里有「张三 / 138xxxx」，模型会把它们填进这一份的
//!    `name` / `phone`。这不是幻觉，是「参考资料里明明有」——必须在指令层
//!    明令禁止，并在渲染层先脱敏。
//! 2. **挤掉正文**：正文已截 8000 字符，记忆条数与单条长度都必须有上限，
//!    否则长记忆会把简历正文推出上下文窗口。
//! 3. **越界抽字段**：模型可能从参考段里抽 `skills` 当成本次结果。指令必须
//!    写明「本段不得作为信息来源，冲突以正文为准，正文缺失仍返回 null」。
//!
//! 阈值都可配（`MEMORY_INJECT_MAX_HITS` / `MEMORY_INJECT_HIT_CHARS`），
//! 但默认值刻意保守：5 条 × 300 字符 ≈ 1500 字符，约占正文预算的 19%。

use mcp_client::MemoryHit;
use regex::Regex;
use std::sync::OnceLock;

/// 默认最多注入几条记忆。与 `recall(..., 5)` 的 k 对齐。
pub const DEFAULT_MAX_HITS: usize = 5;

/// 默认单条记忆保留字符数（按**字符**不按字节，中文不能被切坏）。
pub const DEFAULT_HIT_CHARS: usize = 300;

fn id_card_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b\d{17}[\dXx]\b").expect("身份证正则"))
}

fn phone_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b1[3-9]\d{9}\b").expect("手机号正则"))
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\w.+-]+@[\w-]+\.[\w.-]+").expect("邮箱正则"))
}

/// 注入前脱敏：身份证 → 手机号 → 邮箱。
///
/// 顺序有讲究：身份证和手机号都是纯数字，先清长的，避免 18 位数字被
/// 手机号规则从中间切掉一段后留下残片。只清这三类——**姓名、公司、学校
/// 不用正则硬猜**（中文姓名几乎无法无歧义匹配，猜错会把正文也改坏），
/// 改由 prompt 里的禁令兜住，残余风险已记入 `AB注入阶段-2026-10-17.md`。
pub fn redact(text: &str) -> String {
    let s = id_card_re().replace_all(text, "[已脱敏:证件号]");
    let s = phone_re().replace_all(&s, "[已脱敏:手机号]");
    email_re().replace_all(&s, "[已脱敏:邮箱]").to_string()
}

/// 从一条命中里取出可读文本。
///
/// cortex-mcp 的 `recall` 返回形状未定死（可能是字符串数组，也可能是带
/// `content` 的对象数组），所以按常见键名逐个试，最后才退回整个 JSON。
pub fn hit_text(hit: &MemoryHit) -> Option<String> {
    match hit {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => {
            for key in ["content", "text", "memory", "summary", "body"] {
                if let Some(serde_json::Value::String(s)) = map.get(key) {
                    if !s.trim().is_empty() {
                        return Some(s.clone());
                    }
                }
            }
            None
        }
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// 把命中整理成「可直接拼进 prompt 的若干行」。
///
/// 每行都做：脱敏 → 折成单行（参考段自己的换行会打乱结构）→ 按字符截断。
/// 取一条命中的 id（cortex-mcp 的记录键是 `id`）。
///
/// 为什么要 id 而不是只留文本：`recall` **没有相似度阈值** ——
/// 非空库上必然返回 k 条，所以「命中几条」完全不能说明相关性。
/// 只有把「这次到底注入了哪几条」记下来，事后才能按 id 回查内容，
/// 判断 A/B 的 Δ 是「记忆有用」还是「preload 内容跑偏把模型带歪了」。
pub fn hit_id(hit: &MemoryHit) -> Option<String> {
    hit.get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// 准备注入行，**同时**返回这些行的来源 id（顺序与行一一对应）。
pub fn prepare(
    hits: &[MemoryHit],
    max_hits: usize,
    max_chars: usize,
) -> (Vec<String>, Vec<String>) {
    let max_hits = max_hits.max(1);
    let max_chars = max_chars.max(1);

    let mut lines = Vec::new();
    let mut ids = Vec::new();

    for hit in hits {
        if lines.len() >= max_hits {
            break;
        }
        let Some(raw) = hit_text(hit) else { continue };
        let redacted = redact(&raw);
        let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            continue;
        }
        let line = if normalized.chars().count() > max_chars {
            let mut cut: String = normalized.chars().take(max_chars).collect();
            cut.push('…');
            cut
        } else {
            normalized
        };
        lines.push(line);
        // id 缺失时用空串占位，保持 ids 与 lines 下标对齐
        ids.push(hit_id(hit).unwrap_or_default());
    }

    (lines, ids)
}

/// 只要行、不要 id 的便捷包装（单测与旧调用点用）。
pub fn prepare_lines(hits: &[MemoryHit], max_hits: usize, max_chars: usize) -> Vec<String> {
    prepare(hits, max_hits, max_chars).0
}

/// 渲染注入段。`lines` 为空 ⇒ `None`（调用方据此走**原 prompt**，
/// 保证「开关打开但没命中」与「开关关闭」产出的 prompt 逐字节一致）。
pub fn render_block(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }

    let body = lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}. {}", i + 1, line))
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!(
        r#"【历史处理记录 · 仅供参考】
以下是你在这台终端上处理过的同类文件的摘要，来自本地记忆库。
使用规则（必须遵守）：
- 本段只用于校准提取口径（同类岗位常见的技能写法、字段粒度）。
- 严禁把本段出现的姓名、电话、邮箱、公司名、学校名写入输出。
- 本段不得作为本次文件的字段来源；与正文冲突时一律以正文为准。
- 正文没有的字段仍然返回 null，不要用参考段补全。

{body}
【历史处理记录 · 结束】"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_phone_email_and_id() {
        let raw = "张三 13812345678 zhangsan@example.com 身份证 44030119900101123X";
        let out = redact(raw);
        assert!(!out.contains("13812345678"), "手机号未脱敏: {out}");
        assert!(!out.contains("zhangsan@example.com"), "邮箱未脱敏: {out}");
        assert!(!out.contains("44030119900101123X"), "证件号未脱敏: {out}");
        // 姓名不在正则覆盖范围，保留（由 prompt 禁令兜底）
        assert!(out.contains("张三"));
    }

    /// 脱敏不能把正常业务数字（年份、分数、条数）一起清掉。
    #[test]
    fn redaction_keeps_ordinary_numbers() {
        let raw = "2018-09 至 2022-06，整体质量 75/100，教育 1 项、工作 3 项，社保 12345";
        assert_eq!(redact(raw), raw);
    }

    #[test]
    fn hit_text_reads_known_keys() {
        assert_eq!(hit_text(&json!("plain")), Some("plain".to_string()));
        assert_eq!(
            hit_text(&json!({"content": "c", "kind": "episodic"})),
            Some("c".to_string())
        );
        assert_eq!(hit_text(&json!({"text": "t"})), Some("t".to_string()));
        assert_eq!(hit_text(&json!({"foo": "bar"})), None);
        assert_eq!(hit_text(&json!(null)), None);
    }

    /// 中文截断必须按字符走，按字节切会 panic。
    #[test]
    fn truncates_by_chars_not_bytes() {
        let long = "技".repeat(500);
        let lines = prepare_lines(&[json!({"content": long})], 5, 10);
        assert_eq!(lines.len(), 1);
        let n = lines[0].chars().count();
        assert!(n <= 11, "截断后应 ≤ max_chars+省略号，实际 {n}");
        assert!(lines[0].ends_with('…'));
    }

    #[test]
    fn limits_hit_count() {
        let hits: Vec<_> = (0..10).map(|i| json!({"content": format!("第{i}条")})).collect();
        assert_eq!(prepare_lines(&hits, 3, 300).len(), 3);
    }

    /// 换行必须折平：参考段自带换行会让模型把某一行当成新指令。
    #[test]
    fn flattens_newlines() {
        let lines = prepare_lines(&[json!({"content": "第一行\n第二行\n\n第三行"})], 5, 300);
        assert!(!lines[0].contains('\n'));
        assert_eq!(lines[0], "第一行 第二行 第三行");
    }

    #[test]
    fn empty_hits_render_nothing() {
        assert!(render_block(&[]).is_none());
    }

    /// 渲染段必须同时具备：边界标记、三条禁令、编号正文。
    /// `prepare` 返回的 id 必须与行**下标一一对应**：空白条目被跳过时
    /// 不能占位，缺 id 的条目要用空串占位（否则事后按 id 回查会错位）。
    #[test]
    fn prepare_keeps_ids_aligned_with_lines() {
        use serde_json::json;
        let hits = vec![
            json!({"id": "m-1", "content": "甲"}),
            json!({"content": "乙"}),              // 无 id ⇒ 空串占位
            json!({"id": "m-3", "content": "   "}), // 空白 ⇒ 整条跳过
            json!({"id": "m-4", "content": "丙"}),
        ];
        let (lines, ids) = prepare(&hits, 5, 300);
        assert_eq!(lines.len(), 3);
        assert_eq!(ids, vec!["m-1".to_string(), String::new(), "m-4".to_string()]);
    }

    /// 截断到 max_hits 时，id 也要跟着截（否则 digest 里的 id 比正文多）。
    #[test]
    fn prepare_truncates_ids_with_lines() {
        use serde_json::json;
        let hits: Vec<_> = (1..=5)
            .map(|i| json!({"id": format!("m-{i}"), "content": format!("第{i}条")}))
            .collect();
        let (lines, ids) = prepare(&hits, 2, 300);
        assert_eq!(lines.len(), 2);
        assert_eq!(ids, vec!["m-1".to_string(), "m-2".to_string()]);
    }

    #[test]
    fn block_carries_isolation_rules() {
        let block = render_block(&["技能：Rust、SQL".to_string()]).unwrap();
        assert!(block.contains("【历史处理记录 · 仅供参考】"));
        assert!(block.contains("【历史处理记录 · 结束】"));
        assert!(block.contains("严禁把本段出现的姓名"));
        assert!(block.contains("以正文为准"));
        assert!(block.contains("1. 技能：Rust、SQL"));
    }

    /// 端到端：脏数据进、干净块出。
    #[test]
    fn dirty_hit_never_leaks_pii() {
        let hit = json!({"content": "上传简历「李四.pdf」：电话 13900001111，邮箱 lisi@a.com，技能：Rust"});
        let block = render_block(&prepare_lines(&[hit], 5, 300)).unwrap();
        assert!(!block.contains("13900001111"));
        assert!(!block.contains("lisi@a.com"));
        assert!(block.contains("Rust"));
        assert!(block.contains("[已脱敏:手机号]"));
    }
}
