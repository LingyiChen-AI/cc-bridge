//! 请求统计：定时从代理的请求监控（proxy::monitor）取已完成的请求，给界面用。
//! - 最近 200 条请求只放内存，「动态」页展示；
//! - 按「天 × 线路 × 模型」汇总调用次数 / 出错次数 / 总耗时 / 四项 token，写进 usage.json，保留 400 天；
//! - 首次启动（没有 usage.json）时一次性导入旧的 stats.json（只读，不改不删）。
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::proxy::monitor::{CompletedRequest, EndpointKind, MonitorHandle, RequestStatus};
use crate::proxy::usage_tap::TokenUsage;

const RECENT_MAX: usize = 200;
const RECENT_SHOWN: usize = 100;
const SEEN_MAX: usize = 1000;
const KEEP_DAYS: i64 = 400;
const USE_DAYS: i64 = 7;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    /// 完成时间，Unix 毫秒
    pub at: i64,
    pub model: String,
    /// codex：走 ChatGPT 订阅；claude：原样转发给 Anthropic
    pub route: &'static str,
    pub ok: bool,
    pub http_status: Option<u16>,
    pub ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// 缓存读 token（有精确用量时才有）
    pub cache_read_tokens: Option<u64>,
    /// 缓存写 token（5 分钟 + 1 小时，有精确用量时才有）
    pub cache_write_tokens: Option<u64>,
    pub error: Option<String>,
}

/// 某天某线路某模型的汇总
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Agg {
    /// 请求数
    pub n: u64,
    /// 出错数
    pub e: u64,
    /// 总耗时（毫秒）
    pub ms: u64,
    /// 未命中缓存的输入 token
    #[serde(rename = "in")]
    pub input: u64,
    /// 缓存读
    pub cr: u64,
    /// 缓存写（5 分钟）
    pub cw: u64,
    /// 缓存写（1 小时）
    pub cw1h: u64,
    /// 输出
    pub out: u64,
}

impl Agg {
    /// 记一次请求
    pub fn add_request(&mut self, ok: bool, ms: u64, u: &TokenUsage) {
        self.n += 1;
        self.e += u64::from(!ok);
        self.ms += ms;
        self.input += u.input;
        self.cr += u.cache_read;
        self.cw += u.cache_write_5m;
        self.cw1h += u.cache_write_1h;
        self.out += u.output;
    }

    /// 把另一份汇总加进来
    pub fn merge(&mut self, o: &Agg) {
        self.n += o.n;
        self.e += o.e;
        self.ms += o.ms;
        self.input += o.input;
        self.cr += o.cr;
        self.cw += o.cw;
        self.cw1h += o.cw1h;
        self.out += o.out;
    }

    /// 四项 token
    pub fn usage(&self) -> TokenUsage {
        TokenUsage {
            input: self.input,
            cache_read: self.cr,
            cache_write_5m: self.cw,
            cache_write_1h: self.cw1h,
            output: self.out,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct Persist {
    version: u32,
    /// 日期（本地时区，YYYY-MM-DD）→ 线路（claude / codex / api）→ 模型 → 汇总
    days: BTreeMap<String, BTreeMap<String, BTreeMap<String, Agg>>>,
}

impl Default for Persist {
    fn default() -> Self {
        Self { version: 2, days: BTreeMap::new() }
    }
}

/// 旧 stats.json 的格式：日期 → 模型 → 次数 / 错误 / 耗时
#[derive(Debug, Default, Deserialize)]
struct LegacyDay {
    #[serde(default)]
    count: u64,
    #[serde(default)]
    errors: u64,
    #[serde(default)]
    ms: u64,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyPersist {
    #[serde(default)]
    days: BTreeMap<String, BTreeMap<String, LegacyDay>>,
}

/// 旧数据没记线路，按模型名推断
fn legacy_route(model: &str) -> &'static str {
    if model.starts_with("api-") {
        "api"
    } else if model.starts_with("claude-") {
        "claude"
    } else {
        "codex"
    }
}

fn import_legacy(legacy: &std::path::Path) -> Persist {
    let mut p = Persist::default();
    let Some(old) = std::fs::read(legacy).ok().and_then(|b| serde_json::from_slice::<LegacyPersist>(&b).ok()) else {
        return p;
    };
    for (day, models) in old.days {
        for (model, d) in models {
            let agg = p
                .days
                .entry(day.clone())
                .or_default()
                .entry(legacy_route(&model).to_string())
                .or_default()
                .entry(model)
                .or_default();
            agg.n += d.count;
            agg.e += d.errors;
            agg.ms += d.ms;
        }
    }
    p
}

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Today {
    pub count: u64,
    pub errors: u64,
    pub avg_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub recent: Vec<Activity>,
    pub today: Today,
    /// 近 7 天（含今天）每个模型的调用次数
    pub model_use: BTreeMap<String, u64>,
}

struct Inner {
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    recent: VecDeque<Activity>,
    persist: Persist,
    dirty: bool,
}

pub struct Stats {
    path: PathBuf,
    inner: Mutex<Inner>,
}

fn day_of(t: SystemTime) -> String {
    jiff::Timestamp::try_from(t)
        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()).date().to_string())
        .unwrap_or_default()
}

/// today 往前推 n 天的日期（YYYY-MM-DD），日期字符串可以直接按字典序比较
fn days_before(today: &str, n: i64) -> String {
    today
        .parse::<jiff::civil::Date>()
        .ok()
        .and_then(|d| d.checked_sub(jiff::Span::new().days(n)).ok())
        .map(|d| d.to_string())
        .unwrap_or_default()
}

fn to_activity(r: &CompletedRequest) -> Activity {
    let ok = r.status == RequestStatus::Completed && r.http_status.is_none_or(|s| s < 400);
    Activity {
        id: r.request_id.clone(),
        at: r
            .finished_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
        // 监控里映射过的模型记成「请求的 → 实际的」，统计按 Claude Code 里选的那个（箭头前）
        model: r.model.as_deref().and_then(|m| m.split(" → ").next()).unwrap_or_default().to_string(),
        route: match r.provider.as_deref() {
            Some("anthropic") => "claude",
            Some("custom-api") => "api",
            _ => "codex",
        },
        ok,
        http_status: r.http_status,
        ms: r.latency.as_millis() as u64,
        input_tokens: r.usage.map(|u| u.input).or(r.input_tokens),
        output_tokens: r.output_tokens,
        cache_read_tokens: r.usage.map(|u| u.cache_read),
        cache_write_tokens: r.usage.map(|u| u.cache_write_5m + u.cache_write_1h),
        error: if ok { None } else { r.error.clone() },
    }
}

impl Stats {
    /// 读 usage.json；它不存在时一次性导入旧 stats.json；损坏时改名 .bad 后从空开始（不导入旧文件）。
    pub fn load(path: PathBuf, legacy: PathBuf) -> Self {
        let mut dirty = false;
        let persist = match std::fs::read(&path) {
            Ok(b) => match serde_json::from_slice::<Persist>(&b) {
                Ok(p) => p,
                Err(_) => {
                    let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                    dirty = true;
                    Persist::default()
                }
            },
            Err(_) => {
                let p = import_legacy(&legacy);
                dirty = !p.days.is_empty();
                p
            }
        };
        Self {
            path,
            inner: Mutex::new(Inner {
                seen: HashSet::new(),
                seen_order: VecDeque::new(),
                recent: VecDeque::new(),
                persist,
                dirty,
            }),
        }
    }

    /// 收进监控里新完成的 /v1/messages 请求；有新请求时返回 true。
    pub fn ingest(&self, monitor: &MonitorHandle) -> bool {
        self.ingest_at(monitor, &day_of(SystemTime::now()))
    }

    fn ingest_at(&self, monitor: &MonitorHandle, today: &str) -> bool {
        let mut done: Vec<CompletedRequest> = monitor
            .snapshot()
            .recent
            .into_iter()
            .filter(|r| r.endpoint == EndpointKind::Messages)
            .filter(|r| matches!(r.status, RequestStatus::Completed | RequestStatus::Failed))
            // 没解析出模型就失败的（如模型名不存在）没有真正发到上游，不计入
            .filter(|r| r.model.is_some())
            .collect();
        done.sort_by_key(|r| r.finished_at);
        let mut g = self.inner.lock().unwrap();
        let mut changed = false;
        for r in &done {
            if g.seen.contains(&r.request_id) {
                continue;
            }
            g.seen.insert(r.request_id.clone());
            g.seen_order.push_back(r.request_id.clone());
            if g.seen_order.len() > SEEN_MAX
                && let Some(old) = g.seen_order.pop_front()
            {
                g.seen.remove(&old);
            }
            let a = to_activity(r);
            let day = day_of(r.finished_at);
            let usage = r.usage.unwrap_or_default();
            g.persist
                .days
                .entry(day)
                .or_default()
                .entry(a.route.to_string())
                .or_default()
                .entry(a.model.clone())
                .or_default()
                .add_request(a.ok, a.ms, &usage);
            g.recent.push_front(a);
            g.recent.truncate(RECENT_MAX);
            changed = true;
        }
        if changed {
            let cutoff = days_before(today, KEEP_DAYS);
            g.persist.days.retain(|d, _| d.as_str() > cutoff.as_str());
            g.dirty = true;
        }
        changed
    }

    /// 有改动时写盘（先写临时文件再改名，避免写一半）
    pub fn save(&self) {
        let mut g = self.inner.lock().unwrap();
        if !g.dirty {
            return;
        }
        if let Ok(json) = serde_json::to_vec(&g.persist) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &self.path).is_ok() {
                g.dirty = false;
            }
        }
    }

    pub fn overview(&self) -> Overview {
        self.overview_at(&day_of(SystemTime::now()))
    }

    fn overview_at(&self, today: &str) -> Overview {
        let g = self.inner.lock().unwrap();
        let mut t = Today::default();
        let mut ms = 0;
        if let Some(day) = g.persist.days.get(today) {
            for m in day.values().flat_map(|r| r.values()) {
                t.count += m.n;
                t.errors += m.e;
                ms += m.ms;
            }
        }
        t.avg_ms = if t.count > 0 { ms / t.count } else { 0 };
        let from = days_before(today, USE_DAYS - 1);
        let mut model_use = BTreeMap::new();
        for (d, models) in g.persist.days.range(from..) {
            if d.as_str() > today {
                continue;
            }
            for (m, s) in models.values().flat_map(|r| r.iter()) {
                *model_use.entry(m.clone()).or_insert(0) += s.n;
            }
        }
        Overview { recent: g.recent.iter().take(RECENT_SHOWN).cloned().collect(), today: t, model_use }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor_with(reqs: &[(&str, &str, &str, Result<u16, u16>)]) -> MonitorHandle {
        let m = MonitorHandle::new(50);
        for (id, provider, model, res) in reqs {
            m.request_started(*id, None, None, EndpointKind::Messages);
            m.provider_selected(*id, *provider, *model, None);
            match res {
                Ok(code) => m.request_completed(*id, *code, Some(100), Some(20)),
                Err(code) => m.request_failed(*id, Some(*code), "usage limit reached"),
            }
        }
        m
    }

    fn tmp_path(name: &str) -> PathBuf {
        let dir = tempfile::tempdir().unwrap().keep();
        dir.join(name)
    }

    #[test]
    fn ingests_once_and_counts_per_model() {
        let m = monitor_with(&[
            ("a", "codex", "gpt-6.2-sol", Ok(200)),
            ("b", "anthropic", "claude-opus-5-5", Ok(200)),
            ("c", "codex", "gpt-6.2-sol", Err(429)),
        ]);
        let s = Stats::load(tmp_path("usage.json"), tmp_path("stats.json"));
        assert!(s.ingest(&m));
        assert!(!s.ingest(&m), "同一批请求不能重复计数");
        let o = s.overview();
        assert_eq!(o.recent.len(), 3);
        assert_eq!(o.recent[0].id, "c", "最新的在前");
        assert!(!o.recent[0].ok);
        assert_eq!(o.recent[0].http_status, Some(429));
        assert_eq!(o.recent[1].route, "claude");
        assert_eq!(o.recent[2].route, "codex");
        assert_eq!(o.today.count, 3);
        assert_eq!(o.today.errors, 1);
        assert_eq!(o.model_use.get("gpt-6.2-sol"), Some(&2));
        assert_eq!(o.model_use.get("claude-opus-5-5"), Some(&1));
    }

    #[test]
    fn counts_requested_model_when_mapped() {
        let m = MonitorHandle::new(10);
        m.request_started("a", None, None, EndpointKind::Messages);
        m.provider_selected("a", "codex", "gpt-6.1-sol", None);
        m.model_resolved("a", "gpt-6.1-sol-fast");
        m.request_completed("a", 200, Some(1), Some(1));
        let s = Stats::load(tmp_path("usage.json"), tmp_path("stats.json"));
        s.ingest(&m);
        let o = s.overview();
        let raw = m.snapshot().recent[0].model.clone().unwrap();
        assert!(raw.contains('→'), "监控里是映射格式：{raw}");
        assert_eq!(o.recent[0].model, raw.split(" → ").next().unwrap());
        assert_eq!(o.model_use.len(), 1);
    }

    #[test]
    fn ignores_count_tokens_and_unknown_models() {
        let m = MonitorHandle::new(10);
        m.request_started("x", None, None, EndpointKind::CountTokens);
        m.request_completed("x", 200, None, None);
        m.request_started("y", None, None, EndpointKind::Messages);
        m.request_failed("y", Some(400), "Unknown model");
        let s = Stats::load(tmp_path("usage.json"), tmp_path("stats.json"));
        assert!(!s.ingest(&m));
        assert_eq!(s.overview().today.count, 0);
    }

    #[test]
    fn persists_daily_counts_and_prunes_old_days() {
        let path = tmp_path("usage.json");
        let s = Stats::load(path.clone(), tmp_path("stats.json"));
        s.ingest(&monitor_with(&[("a", "codex", "gpt-6.2-sol", Ok(200))]));
        {
            let mut g = s.inner.lock().unwrap();
            g.persist.days.entry("2000-01-01".into()).or_default().entry("codex".into()).or_default().insert("old".into(), Agg { n: 5, e: 0, ms: 1, ..Default::default() });
        }
        s.ingest(&monitor_with(&[("b", "codex", "gpt-6.2-sol", Ok(200))]));
        s.save();
        let again = Stats::load(path, tmp_path("stats.json"));
        let o = again.overview();
        assert_eq!(o.model_use.get("gpt-6.2-sol"), Some(&2), "重启后从文件恢复");
        assert!(o.recent.is_empty(), "最近请求只在内存里");
        assert!(!again.inner.lock().unwrap().persist.days.contains_key("2000-01-01"), "超过 400 天的删掉");
    }

    #[test]
    fn model_use_covers_last_seven_days_only() {
        let s = Stats::load(tmp_path("usage.json"), tmp_path("stats.json"));
        {
            let mut g = s.inner.lock().unwrap();
            for (d, n) in [("2026-11-18", 1), ("2026-11-12", 2), ("2026-11-11", 4)] {
                g.persist.days.entry(d.into()).or_default().entry("codex".into()).or_default().insert("m".into(), Agg { n, ..Default::default() });
            }
        }
        let o = s.overview_at("2026-11-18");
        assert_eq!(o.model_use.get("m"), Some(&3), "11-12 到 11-18 共 7 天");
        assert_eq!(o.today, Today { count: 1, errors: 0, avg_ms: 0 });
    }

    fn route_agg(s: &Stats, day: &str, route: &str, model: &str) -> Agg {
        let g = s.inner.lock().unwrap();
        g.persist.days.get(day).and_then(|r| r.get(route)).and_then(|m| m.get(model)).cloned().unwrap_or_default()
    }

    #[test]
    fn records_tokens_per_route_and_model() {
        let m = monitor_with(&[
            ("a", "anthropic", "claude-opus-5-5", Ok(200)),
            ("b", "anthropic", "claude-opus-5-5", Ok(200)),
            ("c", "codex", "gpt-6.1-sol", Ok(200)),
        ]);
        for id in ["a", "b"] {
            m.usage_final(id, TokenUsage { cache_read: 100, ..Default::default() });
        }
        let s = Stats::load(tmp_path("usage.json"), tmp_path("stats.json"));
        s.ingest(&m);
        let today = day_of(SystemTime::now());
        let c = route_agg(&s, &today, "claude", "claude-opus-5-5");
        assert_eq!((c.cr, c.n), (200, 2));
        assert_eq!(route_agg(&s, &today, "codex", "gpt-6.1-sol").n, 1);
        let o = s.overview();
        let a = o.recent.iter().find(|a| a.id == "a").unwrap();
        assert_eq!(a.cache_read_tokens, Some(100));
        assert_eq!(a.cache_write_tokens, Some(0));
        let cc = o.recent.iter().find(|a| a.id == "c").unwrap();
        assert_eq!(cc.cache_read_tokens, None);
    }

    #[test]
    fn imports_legacy_stats_json_once() {
        let dir = tempfile::tempdir().unwrap().keep();
        let today = day_of(SystemTime::now());
        let old = days_before(&today, 6);
        let legacy = format!(
            r#"{{"days":{{"{today}":{{"gpt-6.2-sol":{{"count":3,"errors":1,"ms":30}},"api-claude-opus-5-5":{{"count":2,"errors":0,"ms":20}}}},"{old}":{{"claude-opus-5-5":{{"count":4,"errors":0,"ms":40}}}}}}}}"#
        );
        let legacy_path = dir.join("stats.json");
        std::fs::write(&legacy_path, &legacy).unwrap();
        let usage_path = dir.join("usage.json");
        let s = Stats::load(usage_path.clone(), legacy_path.clone());
        let o = s.overview();
        assert_eq!((o.today.count, o.today.errors), (5, 1));
        assert_eq!(o.model_use.get("claude-opus-5-5"), Some(&4));
        assert_eq!(route_agg(&s, &today, "codex", "gpt-6.2-sol").n, 3);
        assert_eq!(route_agg(&s, &today, "api", "api-claude-opus-5-5").n, 2);
        assert_eq!(route_agg(&s, &old, "claude", "claude-opus-5-5").n, 4);
        s.save();
        assert!(usage_path.exists());
        assert_eq!(std::fs::read(&legacy_path).unwrap(), legacy.as_bytes(), "旧文件不能被改动");
        let again = Stats::load(usage_path, legacy_path);
        assert_eq!(again.overview().today.count, 5, "不重复导入");
    }

    #[test]
    fn prunes_days_older_than_400() {
        let s = Stats::load(tmp_path("usage.json"), tmp_path("stats.json"));
        let today = day_of(SystemTime::now());
        let (d400, d399) = (days_before(&today, 400), days_before(&today, 399));
        {
            let mut g = s.inner.lock().unwrap();
            for d in [&d400, &d399] {
                g.persist.days.entry(d.clone()).or_default().entry("codex".into()).or_default().insert("m".into(), Agg { n: 1, ..Default::default() });
            }
        }
        s.ingest(&monitor_with(&[("a", "codex", "gpt-6.2-sol", Ok(200))]));
        let g = s.inner.lock().unwrap();
        assert!(!g.persist.days.contains_key(&d400));
        assert!(g.persist.days.contains_key(&d399));
    }

    #[test]
    fn corrupt_usage_json_falls_back_to_empty() {
        let dir = tempfile::tempdir().unwrap().keep();
        let today = day_of(SystemTime::now());
        let legacy_path = dir.join("stats.json");
        std::fs::write(&legacy_path, format!(r#"{{"days":{{"{today}":{{"m":{{"count":3,"errors":0,"ms":1}}}}}}}}"#)).unwrap();
        let usage_path = dir.join("usage.json");
        std::fs::write(&usage_path, "not json").unwrap();
        let s = Stats::load(usage_path.clone(), legacy_path);
        assert_eq!(s.overview().today.count, 0, "损坏时不导入旧文件");
        assert_eq!(std::fs::read(dir.join("usage.json.bad")).unwrap(), b"not json");
    }
}
