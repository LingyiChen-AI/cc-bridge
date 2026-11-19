//! 下游 Anthropic 协议响应（SSE 或 JSON）的 usage 增量解析器。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub output: u64,
}

impl TokenUsage {
    /// 五项 token 之和。
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write_5m)
            .saturating_add(self.cache_write_1h)
            .saturating_add(self.output)
    }

    pub fn add(&mut self, o: &TokenUsage) {
        self.input = self.input.saturating_add(o.input);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
        self.cache_write_5m = self.cache_write_5m.saturating_add(o.cache_write_5m);
        self.cache_write_1h = self.cache_write_1h.saturating_add(o.cache_write_1h);
        self.output = self.output.saturating_add(o.output);
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// SSE 单行缓冲上限。
const MAX_LINE: usize = 1024 * 1024;
/// 非流式 JSON 累积上限。
const MAX_BODY: usize = 2 * 1024 * 1024;

/// 旁路解析器：只读数据，不影响转发。
pub struct UsageTap {
    sse: bool,
    /// SSE：当前未结束的行；JSON：已累积的响应体
    buf: Vec<u8>,
    /// 当前行超过上限，丢弃到下一个换行
    skipping: bool,
    /// 非流式体超过上限，放弃解析
    overflow: bool,
    usage: TokenUsage,
    seen: bool,
}

impl UsageTap {
    pub fn new(content_type: Option<&str>) -> Self {
        let sse = content_type
            .map(|c| c.to_ascii_lowercase().contains("text/event-stream"))
            .unwrap_or(false);
        Self {
            sse,
            buf: Vec::new(),
            skipping: false,
            overflow: false,
            usage: TokenUsage::default(),
            seen: false,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        if !self.sse {
            if self.overflow {
                return;
            }
            if self.buf.len() + chunk.len() > MAX_BODY {
                self.overflow = true;
                self.buf = Vec::new();
                return;
            }
            self.buf.extend_from_slice(chunk);
            return;
        }
        let mut rest = chunk;
        while !rest.is_empty() {
            match rest.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    if self.skipping {
                        self.skipping = false;
                    } else if self.buf.len() + i <= MAX_LINE {
                        self.buf.extend_from_slice(&rest[..i]);
                        let line = std::mem::take(&mut self.buf);
                        self.handle_line(&line);
                    }
                    self.buf.clear();
                    rest = &rest[i + 1..];
                }
                None => {
                    if !self.skipping {
                        if self.buf.len() + rest.len() > MAX_LINE {
                            self.skipping = true;
                            self.buf.clear();
                        } else {
                            self.buf.extend_from_slice(rest);
                        }
                    }
                    break;
                }
            }
        }
    }

    pub fn finish(mut self) -> Option<TokenUsage> {
        if self.sse {
            // 末行没有换行结尾也要处理
            if !self.skipping && !self.buf.is_empty() {
                let line = std::mem::take(&mut self.buf);
                self.handle_line(&line);
            }
        } else if !self.overflow
            && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&self.buf)
            && let Some(u) = v.get("usage")
        {
            self.apply(u);
        }
        self.seen.then_some(self.usage)
    }

    /// 只解析以 `data:` 开头且含 `"usage"` 的行。
    fn handle_line(&mut self, line: &[u8]) {
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        let Some(data) = text.trim().strip_prefix("data:") else {
            return;
        };
        if !data.contains("\"usage\"") {
            return;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data.trim()) else {
            return;
        };
        // message_start 的 usage 在 message 下，message_delta 的在顶层
        if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
            self.apply(u);
        }
        if let Some(u) = v.get("usage") {
            self.apply(u);
        }
    }

    /// 出现的字段后到覆盖先到；缺失的字段保持原值。
    fn apply(&mut self, u: &serde_json::Value) {
        if !u.is_object() {
            return;
        }
        let num = |k: &str| u.get(k).and_then(|x| x.as_u64());
        if let Some(n) = num("input_tokens") {
            self.usage.input = n;
            self.seen = true;
        }
        if let Some(n) = num("cache_read_input_tokens") {
            self.usage.cache_read = n;
            self.seen = true;
        }
        if let Some(n) = num("output_tokens") {
            self.usage.output = n;
            self.seen = true;
        }
        let split = u.get("cache_creation").filter(|c| c.is_object());
        let t5 = split.and_then(|c| c.get("ephemeral_5m_input_tokens")).and_then(|x| x.as_u64());
        let t1 = split.and_then(|c| c.get("ephemeral_1h_input_tokens")).and_then(|x| x.as_u64());
        if t5.is_some() || t1.is_some() {
            self.usage.cache_write_5m = t5.unwrap_or(0);
            self.usage.cache_write_1h = t1.unwrap_or(0);
            self.seen = true;
        } else if let Some(n) = num("cache_creation_input_tokens") {
            // 无 TTL 细分：全部记为 5 分钟档
            self.usage.cache_write_5m = n;
            self.usage.cache_write_1h = 0;
            self.seen = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SSE: &str = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":3000,\"cache_creation_input_tokens\":200,\"output_tokens\":1}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":55}}\n\n",
    );

    fn sse_tap() -> UsageTap {
        UsageTap::new(Some("text/event-stream; charset=utf-8"))
    }

    #[test]
    fn tap_reads_anthropic_stream() {
        let mut t = sse_tap();
        t.feed(SSE.as_bytes());
        assert_eq!(
            t.finish(),
            Some(TokenUsage {
                input: 12,
                cache_read: 3000,
                cache_write_5m: 200,
                cache_write_1h: 0,
                output: 55
            })
        );
    }

    #[test]
    fn tap_splits_cache_creation_ttl() {
        let s = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"cache_creation_input_tokens\":140,\"cache_creation\":{\"ephemeral_5m_input_tokens\":100,\"ephemeral_1h_input_tokens\":40}}}}\n\n";
        let mut t = sse_tap();
        t.feed(s.as_bytes());
        let u = t.finish().unwrap();
        assert_eq!(u.cache_write_5m, 100);
        assert_eq!(u.cache_write_1h, 40);
    }

    #[test]
    fn tap_parses_usage_split_across_chunks() {
        let bytes = SSE.as_bytes();
        let pos = SSE.find("\"output_tokens\":55").unwrap() + 5;
        let mut t = sse_tap();
        t.feed(&bytes[..pos]);
        t.feed(&bytes[pos..]);
        let mut whole = sse_tap();
        whole.feed(bytes);
        assert_eq!(t.finish(), whole.finish());
        assert_eq!(whole_output(), 55);
    }

    fn whole_output() -> u64 {
        let mut t = sse_tap();
        t.feed(SSE.as_bytes());
        t.finish().unwrap().output
    }

    #[test]
    fn tap_later_value_overrides_estimate() {
        let s = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9000}}}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":120,\"cache_read_input_tokens\":8800,\"output_tokens\":30}}\n\n",
        );
        let mut t = sse_tap();
        t.feed(s.as_bytes());
        let u = t.finish().unwrap();
        assert_eq!(u.input, 120);
        assert_eq!(u.cache_read, 8800);
        assert_eq!(u.output, 30);
    }

    #[test]
    fn tap_reads_non_stream_json() {
        let body = "{\"id\":\"m\",\"type\":\"message\",\"content\":[],\"usage\":{\"input_tokens\":7,\"cache_read_input_tokens\":70,\"cache_creation_input_tokens\":9,\"output_tokens\":3}}";
        let b = body.as_bytes();
        let mut t = UsageTap::new(Some("application/json"));
        t.feed(&b[..10]);
        t.feed(&b[10..60]);
        t.feed(&b[60..]);
        assert_eq!(
            t.finish(),
            Some(TokenUsage {
                input: 7,
                cache_read: 70,
                cache_write_5m: 9,
                cache_write_1h: 0,
                output: 3
            })
        );
    }

    #[test]
    fn tap_without_usage_is_none() {
        let mut t = UsageTap::new(Some("application/json"));
        t.feed(b"{\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"x\"}}");
        assert_eq!(t.finish(), None);
        let mut s = sse_tap();
        s.feed(b"event: ping\ndata: {\"type\":\"ping\"}\n\n");
        assert_eq!(s.finish(), None);
    }
}
