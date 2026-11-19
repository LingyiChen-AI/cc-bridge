//! Claude 等价 API 价格表（美元 / 百万 token）。只有 `claude` 线路计价。

use crate::proxy::usage_tap::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
}

const fn p(input: f64, output: f64, cache_read: f64, cache_write_5m: f64, cache_write_1h: f64) -> Price {
    Price { input, output, cache_read, cache_write_5m, cache_write_1h }
}

/// 价格表：键为规范化后的模型名，完全相等匹配。haiku-5-5 只取 ≤100K 档。
static PRICES: &[(&str, Price)] = &[
    ("claude-opus-5-5", p(4.0, 20.0, 0.2, 5.0, 8.0)),
    ("claude-sonnet-5-5", p(2.0, 10.0, 0.1, 2.5, 4.0)),
    ("claude-sonnet-5", p(2.0, 10.0, 0.2, 2.5, 4.0)),
    ("claude-fable-5-2", p(8.0, 40.0, 0.2, 10.0, 16.0)),
    ("claude-fable-5-1", p(10.0, 50.0, 0.25, 12.5, 20.0)),
    ("claude-opus-5", p(5.0, 25.0, 0.5, 6.25, 10.0)),
    ("claude-haiku-5-5", p(0.1, 0.5, 0.01, 0.125, 0.2)),
    ("claude-haiku-4-5", p(1.0, 5.0, 0.1, 1.25, 2.0)),
    ("claude-opus-4-8", p(5.0, 25.0, 0.5, 6.25, 10.0)),
    ("claude-opus-4-7", p(5.0, 25.0, 0.5, 6.25, 10.0)),
    ("claude-sonnet-4-6", p(3.0, 15.0, 0.3, 3.75, 6.0)),
];

/// 小写 → 去 `[...]` 后缀 → 去 `-YYYYMMDD` / `@YYYYMMDD`。
pub fn normalize_model(model: &str) -> String {
    let mut s = model.trim().to_ascii_lowercase();
    if s.ends_with(']')
        && let Some(i) = s.rfind('[')
    {
        s.truncate(i);
    }
    let b = s.as_bytes();
    if b.len() > 9 {
        let (head, tail) = b.split_at(b.len() - 9);
        if (tail[0] == b'-' || tail[0] == b'@') && tail[1..].iter().all(u8::is_ascii_digit) {
            s.truncate(head.len());
        }
    }
    s
}

/// 只有 `claude` 线路计价；模型名规范化后与键完全相等才命中。
pub fn price_for(route: &str, model: &str) -> Option<&'static Price> {
    if route != "claude" {
        return None;
    }
    let key = normalize_model(model);
    PRICES.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// 等价 API 价（美元）。
pub fn cost_usd(u: &TokenUsage, price: &Price) -> f64 {
    (u.input as f64 * price.input
        + u.cache_read as f64 * price.cache_read
        + u.cache_write_5m as f64 * price.cache_write_5m
        + u.cache_write_1h as f64 * price.cache_write_1h
        + u.output as f64 * price.output)
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_lookup_normalizes_and_matches_exactly() {
        let p = price_for("claude", "claude-sonnet-5-5-20261001").unwrap();
        assert_eq!(p.input, 2.0);
        assert_eq!(p.cache_read, 0.1);
        assert_eq!(price_for("claude", "claude-opus-5-5[1m]").unwrap().output, 20.0);
        assert!(price_for("claude", "Claude-Haiku-4-5@20251001").is_some());
        assert!(price_for("claude", "claude-sonnet-5-6").is_none());
        assert!(price_for("api", "claude-opus-5-5").is_none());
        assert!(price_for("codex", "gpt-6.1-sol").is_none());
    }

    #[test]
    fn cost_counts_each_bucket() {
        let p = price_for("claude", "claude-opus-5-5").unwrap();
        let u = TokenUsage {
            input: 1_000_000,
            cache_read: 1_000_000,
            cache_write_5m: 1_000_000,
            cache_write_1h: 1_000_000,
            output: 1_000_000,
        };
        assert!((cost_usd(&u, p) - (4.0 + 0.2 + 5.0 + 8.0 + 20.0)).abs() < 1e-9);
    }

    #[test]
    fn price_table_matches_spec() {
        // (键, 输入, 输出, 缓存读, 缓存写5m, 缓存写1h)
        let spec: [(&str, f64, f64, f64, f64, f64); 11] = [
            ("claude-opus-5-5", 4.0, 20.0, 0.2, 5.0, 8.0),
            ("claude-sonnet-5-5", 2.0, 10.0, 0.1, 2.5, 4.0),
            ("claude-sonnet-5", 2.0, 10.0, 0.2, 2.5, 4.0),
            ("claude-fable-5-2", 8.0, 40.0, 0.2, 10.0, 16.0),
            ("claude-fable-5-1", 10.0, 50.0, 0.25, 12.5, 20.0),
            ("claude-opus-5", 5.0, 25.0, 0.5, 6.25, 10.0),
            ("claude-haiku-5-5", 0.1, 0.5, 0.01, 0.125, 0.2),
            ("claude-haiku-4-5", 1.0, 5.0, 0.1, 1.25, 2.0),
            ("claude-opus-4-8", 5.0, 25.0, 0.5, 6.25, 10.0),
            ("claude-opus-4-7", 5.0, 25.0, 0.5, 6.25, 10.0),
            ("claude-sonnet-4-6", 3.0, 15.0, 0.3, 3.75, 6.0),
        ];
        for (k, i, o, cr, c5, c1) in spec {
            let p = price_for("claude", k).unwrap_or_else(|| panic!("缺少 {k}"));
            assert_eq!(
                (p.input, p.output, p.cache_read, p.cache_write_5m, p.cache_write_1h),
                (i, o, cr, c5, c1),
                "{k}"
            );
        }
    }
}
