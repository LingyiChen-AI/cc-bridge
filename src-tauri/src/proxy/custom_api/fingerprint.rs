//! 分词指纹：同一段固定文本在各 Claude 型号上的 token 数是固定的，兼容接口背后的其他模型（Qwen、DeepSeek…）
//! 分词器不同，数出来的值对不上。

/// 校验用的固定文本（中英文、数字、符号、重音字母、emoji 混排，放大分词器差异）。
pub const FIXED_TEXT: &str = "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳过了懒狗。 0123456789 !@#$%^&*() \u{e9}\u{e8} \u{1F600}";

#[derive(Clone, Debug, PartialEq)]
pub struct Baseline {
    /// `/v1/messages/count_tokens` 对 `FIXED_TEXT` 返回的 `input_tokens`。
    pub count_tokens: u32,
    /// `/v1/messages` 对 `FIXED_TEXT` 返回的 `usage.input_tokens`（未实测的型号为 None）。
    pub usage_input: Option<u32>,
}

/// (型号前缀, count_tokens, usage_input)
const TABLE: &[(&str, u32, Option<u32>)] = &[
    ("claude-haiku-4-5", 60, None),
    ("claude-sonnet-4-6", 60, None),
    ("claude-sonnet-5", 66, None),
    ("claude-opus-4-8", 66, None),
    ("claude-opus-5", 66, None),
    ("claude-sonnet-5-5", 68, None),
    ("claude-opus-5-5", 68, None),
    ("claude-fable-5-1", 68, None),
    ("claude-opus-4-7", 71, None),
];

/// 去掉结尾的 `-YYYYMMDD`。
fn strip_date_suffix(model: &str) -> &str {
    if let Some((head, tail)) = model.rsplit_once('-')
        && tail.len() == 8
        && tail.bytes().all(|b| b.is_ascii_digit())
    {
        return head;
    }
    model
}

/// 去掉日期后缀后按最长前缀（以 `-` 为边界）查表。
pub fn baseline(model: &str) -> Option<Baseline> {
    let m = strip_date_suffix(model);
    TABLE
        .iter()
        .filter(|(prefix, _, _)| {
            m == *prefix || m.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('-'))
        })
        .max_by_key(|(prefix, _, _)| prefix.len())
        .map(|&(_, count_tokens, usage_input)| Baseline {
            count_tokens,
            usage_input,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_strips_date_suffix() {
        assert_eq!(baseline("claude-haiku-4-5-20251001").unwrap().count_tokens, 60);
        assert!(baseline("claude-opus-9").is_none());
    }

    #[test]
    fn baseline_prefers_longest_prefix() {
        assert_eq!(baseline("claude-sonnet-5").unwrap().count_tokens, 66);
        assert_eq!(baseline("claude-sonnet-5-5").unwrap().count_tokens, 68);
        assert_eq!(baseline("claude-opus-5-5-20260101").unwrap().count_tokens, 68);
        assert_eq!(baseline("claude-opus-4-7").unwrap().count_tokens, 71);
        assert!(baseline("claude-opus-50").is_none());
    }
}
