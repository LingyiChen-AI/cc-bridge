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
///
/// 2026-11-18 经 Claude 订阅（官方 Anthropic）实测：count_tokens 直接测 `FIXED_TEXT`；
/// usage_input 用「Claude Code system 提示 + FIXED_TEXT」同时测 count_tokens 与 messages，
/// 各型号两者均相等，故 usage_input 取 count_tokens 同值。基准不从被测接口取（它不能自证）。
const TABLE: &[(&str, u32, Option<u32>)] = &[
    ("claude-haiku-4-5", 60, Some(60)),
    ("claude-sonnet-4-6", 60, Some(60)),
    ("claude-sonnet-5", 66, Some(66)),
    ("claude-opus-4-8", 66, Some(66)),
    ("claude-opus-5", 66, Some(66)),
    ("claude-haiku-5-5", 68, Some(68)),
    ("claude-sonnet-5-5", 68, Some(68)),
    ("claude-opus-5-5", 68, Some(68)),
    ("claude-fable-5-1", 68, Some(68)),
    ("claude-opus-4-7", 71, Some(71)),
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

    #[test]
    fn baseline_covers_haiku_5_5_with_usage_input() {
        assert_eq!(
            baseline("claude-haiku-5-5"),
            Some(Baseline { count_tokens: 68, usage_input: Some(68) })
        );
        assert_eq!(baseline("claude-opus-4-7").unwrap().usage_input, Some(71));
        assert!(TABLE.iter().all(|(_, _, usage)| usage.is_some()));
    }
}
