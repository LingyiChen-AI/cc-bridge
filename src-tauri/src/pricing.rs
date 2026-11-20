//! 等价 API 价格表（美元 / 百万 token）。`claude` 线路按 Anthropic 价，`codex` 线路按 OpenAI 价；
//! 自定义 API（`api` 线路）不计价。

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

/// OpenAI 官方价（https://developers.openai.com/api/docs/pricing ，2026-11-21 抄录），短上下文（≤272K 输入）。
/// 字段对应：缓存写 5m ← 官方「30m write」，缓存写 1h ← 官方「12h write」；官方写「-」或没列的，写入按输入价计。
/// 模型名的 `-fast` / `-ultrafast` 后缀对应官方 Fast（原 Priority）/ Ultrafast 档。各档不是统一倍数
/// （gpt-5.5 的 Fast 是 2.5 倍，其余多为 2 倍），所以三档逐条照抄，不做换算。
/// gpt-5.3-codex-spark 官方未公布价格，不计价。
/// 统计只按天 × 线路 × 模型聚合，单个请求的输入量没有留存，>272K 的长上下文档（输入 2 倍、输出 1.5 倍）无法区分，统一按短上下文价。
static GPT_STANDARD: &[(&str, Price)] = &[
    ("gpt-6.2-astra", p(8.0, 40.0, 0.4, 10.0, 20.0)),
    ("gpt-6.2-sol", p(2.0, 10.0, 0.1, 2.5, 5.0)),
    ("gpt-6.1-sol", p(2.0, 10.0, 0.1, 2.5, 5.0)),
    ("gpt-6.1-terra", p(1.0, 5.0, 0.05, 1.25, 2.5)),
    ("gpt-6-astra", p(10.0, 50.0, 1.0, 12.5, 25.0)),
    ("gpt-6-sol", p(2.0, 10.0, 0.2, 2.5, 5.0)),
    ("gpt-6-luna", p(0.1, 0.5, 0.01, 0.125, 0.25)),
    ("gpt-5.6-sol", p(4.0, 20.0, 0.4, 5.0, 10.0)),
    ("gpt-5.6-terra", p(2.0, 12.0, 0.2, 2.5, 5.0)),
    ("gpt-5.6-luna", p(0.2, 1.2, 0.02, 0.25, 0.5)),
    ("gpt-5.5", p(5.0, 30.0, 0.5, 5.0, 5.0)),
    ("gpt-5.4", p(2.5, 15.0, 0.25, 2.5, 2.5)),
    ("gpt-5.4-mini", p(0.75, 4.5, 0.075, 0.75, 0.75)),
    ("gpt-5.3-codex", p(1.75, 14.0, 0.175, 1.75, 1.75)),
    ("gpt-5.2", p(1.75, 14.0, 0.175, 1.75, 1.75)),
];

static GPT_FAST: &[(&str, Price)] = &[
    ("gpt-6.2-astra", p(16.0, 80.0, 0.8, 20.0, 40.0)),
    ("gpt-6.2-sol", p(4.0, 20.0, 0.2, 5.0, 10.0)),
    ("gpt-6.1-sol", p(4.0, 20.0, 0.2, 5.0, 10.0)),
    ("gpt-6.1-terra", p(2.0, 10.0, 0.1, 2.5, 5.0)),
    ("gpt-6-astra", p(20.0, 100.0, 2.0, 25.0, 50.0)),
    ("gpt-6-sol", p(4.0, 20.0, 0.4, 5.0, 10.0)),
    ("gpt-6-luna", p(0.2, 1.0, 0.02, 0.25, 0.5)),
    ("gpt-5.6-sol", p(8.0, 40.0, 0.8, 10.0, 20.0)),
    ("gpt-5.6-terra", p(4.0, 24.0, 0.4, 5.0, 10.0)),
    ("gpt-5.6-luna", p(0.4, 2.4, 0.04, 0.5, 1.0)),
    ("gpt-5.5", p(12.5, 75.0, 1.25, 12.5, 12.5)),
    ("gpt-5.4", p(5.0, 30.0, 0.5, 5.0, 5.0)),
    ("gpt-5.4-mini", p(1.5, 9.0, 0.15, 1.5, 1.5)),
    ("gpt-5.3-codex", p(3.5, 28.0, 0.35, 3.5, 3.5)),
    ("gpt-5.2", p(3.5, 28.0, 0.35, 3.5, 3.5)),
];

/// 只列模型清单里实际出现的 ultrafast；gpt-6.2 的 ultrafast 有限时优惠价（到 2027-02-28），清单里暂时没有，不收录。
static GPT_ULTRAFAST: &[(&str, Price)] = &[
    ("gpt-6.1-sol", p(12.0, 60.0, 0.6, 15.0, 30.0)),
    ("gpt-6-astra", p(60.0, 300.0, 6.0, 75.0, 150.0)),
];

fn gpt_price(key: &str) -> Option<&'static Price> {
    let (base, table) = if let Some(b) = key.strip_suffix("-ultrafast") {
        (b, GPT_ULTRAFAST)
    } else if let Some(b) = key.strip_suffix("-fast") {
        (b, GPT_FAST)
    } else {
        (key, GPT_STANDARD)
    };
    table.iter().find(|(k, _)| *k == base).map(|(_, v)| v)
}

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

/// `claude` 与 `codex` 线路计价；模型名规范化后与键完全相等才命中。
pub fn price_for(route: &str, model: &str) -> Option<&'static Price> {
    let key = normalize_model(model);
    match route {
        "claude" => PRICES.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
        "codex" => gpt_price(&key),
        _ => None,
    }
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
        assert!(price_for("codex", "claude-opus-5-5").is_none());
    }

    #[test]
    fn gpt_tiers_by_suffix() {
        let std = price_for("codex", "gpt-6.1-sol").unwrap();
        assert_eq!((std.input, std.cache_read, std.output), (2.0, 0.1, 10.0));
        let fast = price_for("codex", "gpt-6.1-sol-fast").unwrap();
        assert_eq!((fast.input, fast.cache_read, fast.output), (4.0, 0.2, 20.0));
        let uf = price_for("codex", "GPT-6.1-Sol-Ultrafast").unwrap();
        assert_eq!((uf.input, uf.cache_read, uf.output), (12.0, 0.6, 60.0));
        // gpt-5.5 的 Fast 不是 2 倍
        assert_eq!(price_for("codex", "gpt-5.5-fast").unwrap().output, 75.0);
        // 官方没价的保持不计价
        assert!(price_for("codex", "gpt-5.3-codex-spark").is_none());
        assert!(price_for("codex", "gpt-5.3-codex-spark-fast").is_none());
        // 没有 ultrafast 价的不能退回到别的档
        assert!(price_for("codex", "gpt-6.2-astra-ultrafast").is_none());
        // 其它线路不按 GPT 价算
        assert!(price_for("claude", "gpt-6.1-sol").is_none());
        assert!(price_for("api", "gpt-6.1-sol").is_none());
    }

    /// 当前模型清单里除 spark 外全部要有价，漏一个统计页就会冒「未知价格」提示。
    #[test]
    fn every_listed_gpt_model_is_priced() {
        let listed = [
            "gpt-5.2", "gpt-5.2-fast", "gpt-5.3-codex", "gpt-5.3-codex-fast", "gpt-5.4", "gpt-5.4-fast", "gpt-5.4-mini",
            "gpt-5.4-mini-fast", "gpt-5.5", "gpt-5.5-fast", "gpt-5.6-luna", "gpt-5.6-luna-fast", "gpt-5.6-sol", "gpt-5.6-sol-fast",
            "gpt-5.6-terra", "gpt-5.6-terra-fast", "gpt-6-astra", "gpt-6-astra-fast", "gpt-6-astra-ultrafast", "gpt-6-luna",
            "gpt-6-luna-fast", "gpt-6-sol", "gpt-6-sol-fast", "gpt-6.1-sol", "gpt-6.1-sol-fast", "gpt-6.1-sol-ultrafast",
            "gpt-6.1-terra", "gpt-6.1-terra-fast", "gpt-6.2-astra", "gpt-6.2-astra-fast", "gpt-6.2-sol", "gpt-6.2-sol-fast",
        ];
        for m in listed {
            assert!(price_for("codex", m).is_some(), "{m} 没有价格");
        }
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
