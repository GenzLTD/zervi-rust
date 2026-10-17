//! serde 兼容层：把模型的「显式 null」当「缺失」处理。
//!
//! ## 为什么需要
//!
//! `#[serde(default)]` 只兜**字段缺失**，兜不住**显式 `null`**。qwen2.5 会把不认识
//! 的字段写成 `null`（实测 `"major": null`），而 `Education.school/major/degree`、
//! `WorkExperience.company/position/description` 都是 `String` ⇒
//! `invalid type: null, expected a string`，**整个 /upload 直接 500**。
//!
//! qwen2.5:3b 必挂，7b 能过但仍会出现 null。这不是「模型偶尔抽风」，是本地小模型
//! 的常态输出，所以按常态处理，而不是等它写出规范 JSON。
//!
//! ## 用法
//!
//! ```ignore
//! #[serde(default, deserialize_with = "null_to_default")]
//! pub school: String,
//! ```
//!
//! `default` 负责「缺失」，`deserialize_with` 负责「显式 null」，两个都要。

use serde::Deserialize;

/// 把 `null` 反序列化为 `T::default()`；非 null 时正常解析。
pub fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Sample {
        #[serde(default, deserialize_with = "null_to_default")]
        text: String,
        #[serde(default, deserialize_with = "null_to_default")]
        score: i32,
        #[serde(default, deserialize_with = "null_to_default")]
        items: Vec<String>,
    }

    #[test]
    fn explicit_null_becomes_default() {
        let s: Sample = serde_json::from_str(r#"{"text": null, "score": null, "items": null}"#)
            .expect("显式 null 不应报错");
        assert_eq!(s.text, "");
        assert_eq!(s.score, 0);
        assert!(s.items.is_empty());
    }

    #[test]
    fn missing_field_still_uses_default() {
        let s: Sample = serde_json::from_str("{}").expect("缺失字段不应报错");
        assert_eq!(s.text, "");
        assert_eq!(s.score, 0);
    }

    #[test]
    fn real_values_pass_through() {
        let s: Sample =
            serde_json::from_str(r#"{"text": "ok", "score": 42, "items": ["a"]}"#).unwrap();
        assert_eq!(s.text, "ok");
        assert_eq!(s.score, 42);
        assert_eq!(s.items, vec!["a".to_string()]);
    }
}
