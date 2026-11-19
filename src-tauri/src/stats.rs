//! 请求统计：定时从代理的请求监控（proxy::monitor）取已完成的请求，给界面用。
//! - 最近 200 条请求只放内存，「动态」页展示；
//! - 按「天 × 线路 × 模型」汇总调用次数 / 出错次数 / 总耗时 / 四项 token，写进 usage.json，保留 400 天；
//! - 首次启动（没有 usage.json）时一次性导入旧的 stats.json（只读，不改不删）。
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

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

/// 坏文件的备份名：usage.json.bad，已存在就依次用 .bad.1、.bad.2……，不覆盖旧备份
fn unique_bad_path(path: &std::path::Path) -> PathBuf {
    let first = path.with_extension("json.bad");
    if !first.exists() {
        return first;
    }
    (1u32..)
        .map(|n| path.with_extension(format!("json.bad.{n}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn import_legacy(legacy: &std::path::Path) -> Persist {
    let mut p = Persist::default();
    let bytes = match std::fs::read(legacy) {
        Ok(b) => b,
        Err(_) => return p,
    };
    let Ok(old) = serde_json::from_slice::<LegacyPersist>(&bytes) else {
        tracing::warn!("旧 stats.json 解析失败，跳过导入");
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

/// 报表的时间粒度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Day,
    Month,
}

impl Granularity {
    /// 只认 "day" / "month"
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "day" => Ok(Self::Day),
            "month" => Ok(Self::Month),
            other => Err(format!("不支持的粒度：{other}")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Month => "month",
        }
    }
}

/// 报表查询条件：日期为本地时区的 YYYY-MM-DD
#[derive(Debug, Clone, Deserialize)]
pub struct ReportQuery {
    pub from: String,
    pub to: String,
    pub granularity: Granularity,
    pub route: Option<String>,
    pub model: Option<String>,
}

impl ReportQuery {
    /// 从界面传来的字符串构造：日期必须是合法的 YYYY-MM-DD，粒度只认 day / month，否则返回 Err
    pub fn parse(
        from: &str,
        to: &str,
        granularity: &str,
        route: Option<String>,
        model: Option<String>,
    ) -> Result<Self, String> {
        let date = |s: &str| {
            s.parse::<jiff::civil::Date>().map(|d| d.to_string()).map_err(|_| format!("日期格式错误：{s}"))
        };
        Ok(Self { from: date(from)?, to: date(to)?, granularity: Granularity::parse(granularity)?, route, model })
    }
}

/// 一组汇总数；`cost_usd` 是按价格表折算的「等价 API 价」，没有价格的模型不计入
#[derive(Debug, Default, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub count: u64,
    pub errors: u64,
    pub ms: u64,
    /// 未命中缓存的输入
    pub input: u64,
    pub cache_read: u64,
    /// 缓存写（5 分钟 + 1 小时）
    pub cache_write: u64,
    pub output: u64,
    pub cost_usd: f64,
}

impl Totals {
    fn add_agg(&mut self, a: &Agg, cost: f64) {
        self.add(&Totals {
            count: a.n,
            errors: a.e,
            ms: a.ms,
            input: a.input,
            cache_read: a.cr,
            cache_write: a.cw + a.cw1h,
            output: a.out,
            cost_usd: cost,
        });
    }

    fn add(&mut self, o: &Totals) {
        self.count += o.count;
        self.errors += o.errors;
        self.ms += o.ms;
        self.input += o.input;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.output += o.output;
        self.cost_usd += o.cost_usd;
    }

    fn tokens(&self) -> u64 {
        self.input + self.cache_read + self.cache_write + self.output
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub route: String,
    pub model: String,
    pub totals: Totals,
    /// 有价格表条目（金额才有意义）
    pub priced: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// 日桶 YYYY-MM-DD，月桶 YYYY-MM
    pub key: String,
    pub by_route: BTreeMap<String, Totals>,
    /// 按金额降序、再按 token 总量降序
    pub models: Vec<ModelRow>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    /// 夹取后的实际范围
    pub from: String,
    pub to: String,
    pub granularity: String,
    /// 范围天数（含空天）
    pub days: u32,
    pub totals: Totals,
    pub by_route: BTreeMap<String, Totals>,
    /// 只含有数据的桶，空桶由前端补零
    pub buckets: Vec<Bucket>,
    /// 有请求或 token 但没有价格的模型名（去重、字典序）
    pub unpriced_models: Vec<String>,
    /// 范围内出现过的全部线路 / 模型（不受 route / model 过滤影响，给筛选下拉用）
    pub routes: Vec<String>,
    pub models: Vec<String>,
}

/// 夹取后的范围与天数
fn clamp_range(q: &ReportQuery, today: &str) -> (String, String, u32) {
    let today_d: jiff::civil::Date = today.parse().unwrap_or_default();
    let parse = |s: &str| s.parse::<jiff::civil::Date>().unwrap_or(today_d);
    let (mut from, mut to) = (parse(&q.from), parse(&q.to));
    if from > to {
        std::mem::swap(&mut from, &mut to);
    }
    let earliest = today_d.checked_sub(jiff::Span::new().days(KEEP_DAYS - 1)).unwrap_or(today_d);
    from = from.max(earliest).min(today_d);
    to = to.min(today_d).max(from);
    let days = (to - from).get_days().max(0) as u32 + 1;
    (from.to_string(), to.to_string(), days)
}

/// CSV 字段：含逗号 / 引号 / 换行时加引号
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

struct Inner {
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    recent: VecDeque<Activity>,
    persist: Persist,
    dirty: bool,
    /// 磁盘上的 usage.json 状态不明（读不了 / 坏文件没能备份）时置位，本次运行不写盘，避免覆盖原文件
    persist_disabled: bool,
    /// 上次尝试写盘的时间；节流写盘按它算间隔
    last_save: Option<Instant>,
}

/// 脏数据最多每隔这么久写一次盘；退出时另外补写一次
const SAVE_INTERVAL: Duration = Duration::from_secs(30);

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
        let mut persist_disabled = false;
        let persist = match std::fs::read(&path) {
            Ok(b) => match serde_json::from_slice::<Persist>(&b) {
                Ok(p) => p,
                Err(e) => {
                    // 坏文件先备份（不覆盖已有备份）再从空开始；备份失败就不落盘，免得空数据把它盖掉
                    let bad = unique_bad_path(&path);
                    match std::fs::rename(&path, &bad) {
                        Ok(()) => {
                            tracing::warn!("usage.json 损坏（{e}），已备份为 {}，从空开始", bad.display());
                            dirty = true;
                        }
                        Err(re) => {
                            tracing::warn!("usage.json 损坏（{e}）且备份失败（{re}），本次运行不写盘");
                            persist_disabled = true;
                        }
                    }
                    Persist::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let p = import_legacy(&legacy);
                dirty = !p.days.is_empty();
                p
            }
            Err(e) => {
                // 暂时读不了（被占用 / 没权限）：既不导入旧文件，也不落盘
                tracing::warn!("读取 usage.json 失败（{e}），本次运行不写盘");
                persist_disabled = true;
                Persist::default()
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
                persist_disabled,
                last_save: None,
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

    /// 有改动时立即写盘（退出时补写用）
    pub fn save(&self) {
        let mut g = self.inner.lock().unwrap();
        self.write_locked(&mut g, Instant::now());
    }

    /// 节流写盘：有改动且距上次写盘满 SAVE_INTERVAL（或从没写过）才写；返回本次是否尝试了写盘
    pub fn save_throttled(&self) -> bool {
        self.save_throttled_at(Instant::now())
    }

    fn save_throttled_at(&self, now: Instant) -> bool {
        let mut g = self.inner.lock().unwrap();
        if !g.dirty || g.persist_disabled {
            return false;
        }
        if g.last_save.is_some_and(|t| now.saturating_duration_since(t) < SAVE_INTERVAL) {
            return false;
        }
        self.write_locked(&mut g, now);
        true
    }

    /// 先写临时文件再改名，避免写一半；失败打 warn，保留 dirty 等下次重试
    fn write_locked(&self, g: &mut Inner, now: Instant) {
        if !g.dirty || g.persist_disabled {
            return;
        }
        // 成败都记时间：写失败时也按间隔重试，不会每 2 秒刷一次 warn
        g.last_save = Some(now);
        let json = match serde_json::to_vec(&g.persist) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("序列化 usage.json 失败：{e}");
                return;
            }
        };
        let tmp = self.path.with_extension("json.tmp");
        let res = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, &self.path));
        match res {
            Ok(()) => g.dirty = false,
            Err(e) => tracing::warn!("写入 {} 失败：{e}", self.path.display()),
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

    /// 范围内按「桶 × 线路 × 模型」归并（已套用线路 / 模型过滤）；同时返回范围内全部线路与模型
    #[allow(clippy::type_complexity)]
    fn collect(
        &self,
        q: &ReportQuery,
        from: &str,
        to: &str,
    ) -> (BTreeMap<(String, String, String), Agg>, BTreeSet<String>, BTreeSet<String>) {
        let g = self.inner.lock().unwrap();
        let mut rows: BTreeMap<(String, String, String), Agg> = BTreeMap::new();
        let (mut routes, mut models) = (BTreeSet::new(), BTreeSet::new());
        for (day, by_route) in g.persist.days.range::<str, _>((Bound::Included(from), Bound::Included(to))) {
            let key = match q.granularity {
                Granularity::Day => day.clone(),
                Granularity::Month => day.get(..7).unwrap_or(day).to_string(),
            };
            for (route, by_model) in by_route {
                for (model, a) in by_model {
                    routes.insert(route.clone());
                    models.insert(model.clone());
                    if q.route.as_deref().is_some_and(|r| r != route) || q.model.as_deref().is_some_and(|m| m != model) {
                        continue;
                    }
                    rows.entry((key.clone(), route.clone(), model.clone())).or_default().merge(a);
                }
            }
        }
        (rows, routes, models)
    }

    /// 某线路某模型的等价 API 价；没有价格返回 None
    fn cost_of(route: &str, model: &str, a: &Agg) -> Option<f64> {
        crate::pricing::price_for(route, model).map(|p| crate::pricing::cost_usd(&a.usage(), p))
    }

    pub fn report(&self, q: &ReportQuery) -> UsageReport {
        self.report_at(q, &day_of(SystemTime::now()))
    }

    fn report_at(&self, q: &ReportQuery, today: &str) -> UsageReport {
        let (from, to, days) = clamp_range(q, today);
        let (rows, routes, models) = self.collect(q, &from, &to);
        let mut totals = Totals::default();
        let mut by_route: BTreeMap<String, Totals> = BTreeMap::new();
        let mut unpriced = BTreeSet::new();
        let mut buckets: Vec<Bucket> = Vec::new();
        // rows 按 (桶, 线路, 模型) 有序，同桶的行相邻
        for ((key, route, model), a) in &rows {
            let cost = Self::cost_of(route, model, a);
            if cost.is_none() && (a.n > 0 || a.input + a.cr + a.cw + a.cw1h + a.out > 0) {
                unpriced.insert(model.clone());
            }
            let mut t = Totals::default();
            t.add_agg(a, cost.unwrap_or(0.0));
            totals.add(&t);
            by_route.entry(route.clone()).or_default().add(&t);
            if buckets.last().is_none_or(|b| &b.key != key) {
                buckets.push(Bucket { key: key.clone(), by_route: BTreeMap::new(), models: Vec::new() });
            }
            let b = buckets.last_mut().unwrap();
            b.by_route.entry(route.clone()).or_default().add(&t);
            b.models.push(ModelRow { route: route.clone(), model: model.clone(), totals: t, priced: cost.is_some() });
        }
        for b in &mut buckets {
            b.models.sort_by(|x, y| {
                y.totals
                    .cost_usd
                    .total_cmp(&x.totals.cost_usd)
                    .then(y.totals.tokens().cmp(&x.totals.tokens()))
                    .then(x.model.cmp(&y.model))
            });
        }
        UsageReport {
            from,
            to,
            granularity: q.granularity.as_str().to_string(),
            days,
            totals,
            by_route,
            buckets,
            unpriced_models: unpriced.into_iter().collect(),
            routes: routes.into_iter().collect(),
            models: models.into_iter().collect(),
        }
    }

    /// 导出 CSV 到 dir：UTF-8 BOM、金额 6 位小数、未知价格留空；返回文件路径
    pub fn export_csv(&self, q: &ReportQuery, dir: &Path) -> std::io::Result<PathBuf> {
        let (from, to, _) = clamp_range(q, &day_of(SystemTime::now()));
        let (rows, _, _) = self.collect(q, &from, &to);
        let mut out = String::from("\u{feff}");
        out.push_str(
            "period,route,model,requests,errors,avg_ms,input_uncached,cache_read,cache_write_5m,cache_write_1h,output,cost_usd\n",
        );
        for ((key, route, model), a) in &rows {
            let cost = Self::cost_of(route, model, a).map(|c| format!("{c:.6}")).unwrap_or_default();
            let avg = a.ms.checked_div(a.n).unwrap_or(0);
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(key),
                csv_field(route),
                csv_field(model),
                a.n,
                a.e,
                avg,
                a.input,
                a.cr,
                a.cw,
                a.cw1h,
                a.out,
                cost
            ));
        }
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("cc-bridge-usage-{from}_{to}.csv"));
        std::fs::write(&path, out)?;
        Ok(path)
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

    #[test]
    fn save_throttled_writes_at_most_every_interval() {
        let dir = std::env::temp_dir().join(format!("ccb-stats-throttle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("usage.json");
        let s = Stats::load(path.clone(), dir.join("stats.json"));
        let t0 = Instant::now();
        // 磁盘上 usage.json 记的今日请求数
        let on_disk = || Stats::load(path.clone(), dir.join("stats.json")).overview().today.count;
        assert!(!s.save_throttled_at(t0), "没有改动不写");

        s.ingest(&monitor_with(&[("a", "anthropic", "claude-opus-5-5", Ok(200))]));
        assert!(s.save_throttled_at(t0), "从没写过：有改动立即写");
        assert_eq!(on_disk(), 1);

        s.ingest(&monitor_with(&[("b", "anthropic", "claude-opus-5-5", Ok(200))]));
        assert!(!s.save_throttled_at(t0 + Duration::from_secs(29)), "30 秒内不再写");
        assert_eq!(on_disk(), 1);
        assert!(s.save_throttled_at(t0 + Duration::from_secs(30)), "满 30 秒写入");
        assert_eq!(on_disk(), 2);
        assert!(!s.save_throttled_at(t0 + Duration::from_secs(90)), "写完不脏就不写");

        // 退出补写：节流窗口内也立即写
        s.ingest(&monitor_with(&[("c", "anthropic", "claude-opus-5-5", Ok(200))]));
        assert!(!s.save_throttled_at(t0 + Duration::from_secs(31)));
        s.save();
        assert_eq!(on_disk(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows 没有 /usr/share/zoneinfo，必须靠 tzdb-bundle-platform 打包的时区库；
    /// 否则 TimeZone::system() 回退 Etc/Unknown（≈UTC），统计日期在北京 00:00–08:00 会错一天
    #[cfg(windows)]
    #[test]
    fn windows_system_time_zone_resolves() {
        let tz = jiff::tz::TimeZone::try_system();
        assert!(tz.is_ok(), "系统时区解析失败：{:?}", tz.err());
        assert_ne!(tz.unwrap().iana_name(), Some("Etc/Unknown"));
    }

    #[test]
    fn ingests_once_and_counts_per_model() {
        let m = monitor_with(&[
            ("a", "codex", "gpt-6.2-sol", Ok(200)),
            ("b", "anthropic", "claude-opus-5-5", Ok(200)),
            ("c", "codex", "gpt-6.2-sol", Err(429)),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let s = Stats::load(tmp.path().join("usage.json"), tmp.path().join("stats.json"));
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
        let tmp = tempfile::tempdir().unwrap();
        let s = Stats::load(tmp.path().join("usage.json"), tmp.path().join("stats.json"));
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
        let tmp = tempfile::tempdir().unwrap();
        let s = Stats::load(tmp.path().join("usage.json"), tmp.path().join("stats.json"));
        assert!(!s.ingest(&m));
        assert_eq!(s.overview().today.count, 0);
    }

    #[test]
    fn persists_daily_counts_and_prunes_old_days() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("usage.json");
        let s = Stats::load(path.clone(), tmp.path().join("stats.json"));
        s.ingest(&monitor_with(&[("a", "codex", "gpt-6.2-sol", Ok(200))]));
        {
            let mut g = s.inner.lock().unwrap();
            g.persist.days.entry("2000-01-01".into()).or_default().entry("codex".into()).or_default().insert("old".into(), Agg { n: 5, e: 0, ms: 1, ..Default::default() });
        }
        s.ingest(&monitor_with(&[("b", "codex", "gpt-6.2-sol", Ok(200))]));
        s.save();
        let again = Stats::load(path, tmp.path().join("stats.json"));
        let o = again.overview();
        assert_eq!(o.model_use.get("gpt-6.2-sol"), Some(&2), "重启后从文件恢复");
        assert!(o.recent.is_empty(), "最近请求只在内存里");
        assert!(!again.inner.lock().unwrap().persist.days.contains_key("2000-01-01"), "超过 400 天的删掉");
    }

    #[test]
    fn model_use_covers_last_seven_days_only() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Stats::load(tmp.path().join("usage.json"), tmp.path().join("stats.json"));
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
        let tmp = tempfile::tempdir().unwrap();
        let s = Stats::load(tmp.path().join("usage.json"), tmp.path().join("stats.json"));
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
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
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
        let tmp = tempfile::tempdir().unwrap();
        let s = Stats::load(tmp.path().join("usage.json"), tmp.path().join("stats.json"));
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
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let today = day_of(SystemTime::now());
        let legacy_path = dir.join("stats.json");
        std::fs::write(&legacy_path, format!(r#"{{"days":{{"{today}":{{"m":{{"count":3,"errors":0,"ms":1}}}}}}}}"#)).unwrap();
        let usage_path = dir.join("usage.json");
        std::fs::write(&usage_path, "not json").unwrap();
        let s = Stats::load(usage_path.clone(), legacy_path);
        assert_eq!(s.overview().today.count, 0, "损坏时不导入旧文件");
        assert_eq!(std::fs::read(dir.join("usage.json.bad")).unwrap(), b"not json");
    }

    #[test]
    fn corrupt_backup_never_overwrites_existing_bad() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("usage.json.bad"), "first").unwrap();
        std::fs::write(dir.join("usage.json"), "second corrupt").unwrap();
        let s = Stats::load(dir.join("usage.json"), dir.join("stats.json"));
        assert_eq!(s.overview().today.count, 0);
        assert_eq!(std::fs::read(dir.join("usage.json.bad")).unwrap(), b"first", "旧备份不能丢");
        assert_eq!(std::fs::read(dir.join("usage.json.bad.1")).unwrap(), b"second corrupt");
    }

    #[test]
    fn unreadable_usage_json_does_not_import_or_save() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let today = day_of(SystemTime::now());
        std::fs::write(dir.join("stats.json"), format!(r#"{{"days":{{"{today}":{{"m":{{"count":3,"errors":0,"ms":1}}}}}}}}"#)).unwrap();
        // 把 usage.json 建成目录：读它会得到非 NotFound 的错误
        std::fs::create_dir(dir.join("usage.json")).unwrap();
        let s = Stats::load(dir.join("usage.json"), dir.join("stats.json"));
        assert_eq!(s.overview().today.count, 0, "读不了时不导入旧文件");
        s.ingest(&monitor_with(&[("a", "codex", "gpt-6.2-sol", Ok(200))]));
        s.save();
        assert!(dir.join("usage.json").is_dir(), "不落盘，原路径不被覆盖");
        assert!(!dir.join("usage.json.tmp").exists());
    }

    // ---------- 报表 ----------

    /// 直接往汇总里放一条
    fn put(s: &Stats, day: &str, route: &str, model: &str, a: Agg) {
        let mut g = s.inner.lock().unwrap();
        g.persist.days.entry(day.into()).or_default().entry(route.into()).or_default().insert(model.into(), a);
    }

    fn agg(n: u64, input: u64, out: u64) -> Agg {
        Agg { n, input, out, ..Default::default() }
    }

    fn empty_stats() -> (tempfile::TempDir, Stats) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Stats::load(tmp.path().join("usage.json"), tmp.path().join("stats.json"));
        (tmp, s)
    }

    fn q(from: &str, to: &str, g: Granularity) -> ReportQuery {
        ReportQuery { from: from.into(), to: to.into(), granularity: g, route: None, model: None }
    }

    #[test]
    fn report_sums_cost_for_priced_models() {
        let (_t, s) = empty_stats();
        put(&s, "2026-11-19", "claude", "claude-opus-5-5", agg(1, 1_000_000, 1_000_000));
        let r = s.report_at(&q("2026-11-19", "2026-11-19", Granularity::Day), "2026-11-19");
        assert_eq!(r.totals.cost_usd, 24.0);
        assert_eq!(r.by_route["claude"].cost_usd, 24.0);
        assert!(r.unpriced_models.is_empty());
        assert!(r.buckets[0].models[0].priced);
    }

    #[test]
    fn api_and_codex_routes_are_unpriced() {
        let (_t, s) = empty_stats();
        put(&s, "2026-11-19", "claude", "claude-opus-5-5", agg(1, 1_000_000, 1_000_000));
        put(&s, "2026-11-19", "api", "api-claude-opus-5-5", agg(1, 10, 20));
        put(&s, "2026-11-18", "codex", "gpt-6.1-sol", agg(2, 100, 200));
        put(&s, "2026-11-19", "codex", "gpt-6.1-sol", agg(1, 1, 2));
        let r = s.report_at(&q("2026-11-18", "2026-11-19", Granularity::Day), "2026-11-19");
        assert_eq!(r.totals.cost_usd, 24.0);
        assert_eq!(r.totals.input, 1_000_000 + 10 + 100 + 1);
        assert_eq!(r.unpriced_models, vec!["api-claude-opus-5-5", "gpt-6.1-sol"]);
    }

    #[test]
    fn report_clamps_range_and_buckets_by_month() {
        let (_t, s) = empty_stats();
        put(&s, "2026-10-30", "claude", "claude-opus-5-5", agg(1, 1, 1));
        put(&s, "2026-11-19", "claude", "claude-opus-5-5", agg(1, 1, 1));
        // 倒序 + 按月
        let r = s.report_at(&q("2026-11-19", "2026-10-25", Granularity::Month), "2026-11-19");
        assert_eq!(r.from, "2026-10-25");
        assert_eq!(r.to, "2026-11-19");
        assert_eq!(r.days, 26);
        let keys: Vec<_> = r.buckets.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(keys, ["2026-10", "2026-11"]);
        assert_eq!(r.granularity, "month");
        // from 太早：夹到 今天 − 399 天；to 太晚：夹到今天
        let r = s.report_at(&q("2020-01-01", "2030-01-01", Granularity::Day), "2026-11-19");
        assert_eq!(r.from, "2025-10-16");
        assert_eq!(r.to, "2026-11-19");
        assert_eq!(r.days, 400);
    }

    #[test]
    fn report_filters_route_and_model_but_lists_all_options() {
        let (_t, s) = empty_stats();
        put(&s, "2026-11-19", "claude", "claude-opus-5-5", agg(1, 5, 5));
        put(&s, "2026-11-19", "codex", "gpt-6.1-sol", agg(3, 7, 7));
        let mut query = q("2026-11-19", "2026-11-19", Granularity::Day);
        query.route = Some("codex".into());
        let r = s.report_at(&query, "2026-11-19");
        assert_eq!(r.totals.count, 3);
        assert_eq!(r.totals.input, 7);
        assert!(!r.by_route.contains_key("claude"));
        assert_eq!(r.routes, vec!["claude", "codex"]);
        assert_eq!(r.models, vec!["claude-opus-5-5", "gpt-6.1-sol"]);
        // 按模型过滤
        let mut query = q("2026-11-19", "2026-11-19", Granularity::Day);
        query.model = Some("claude-opus-5-5".into());
        let r = s.report_at(&query, "2026-11-19");
        assert_eq!(r.totals.count, 1);
        assert_eq!(r.buckets[0].models.len(), 1);
    }

    #[test]
    fn cache_hit_inputs_are_separate() {
        let (_t, s) = empty_stats();
        put(&s, "2026-11-19", "claude", "claude-opus-5-5", Agg { n: 1, input: 10, cr: 20, cw: 30, cw1h: 40, out: 50, ..Default::default() });
        let r = s.report_at(&q("2026-11-19", "2026-11-19", Granularity::Day), "2026-11-19");
        assert_eq!((r.totals.input, r.totals.cache_read, r.totals.cache_write, r.totals.output), (10, 20, 70, 50));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["totals"]["cacheRead"], 20);
        assert_eq!(v["byRoute"]["claude"]["cacheWrite"], 70);
        assert!(v["unpricedModels"].is_array());
    }

    #[test]
    fn models_sorted_by_cost_then_tokens() {
        let (_t, s) = empty_stats();
        put(&s, "2026-11-19", "codex", "gpt-6.1-sol", agg(1, 9_000_000, 0));
        put(&s, "2026-11-19", "claude", "claude-sonnet-5", agg(1, 1_000_000, 0));
        put(&s, "2026-11-19", "claude", "claude-opus-5-5", agg(1, 1_000_000, 0));
        let r = s.report_at(&q("2026-11-19", "2026-11-19", Granularity::Day), "2026-11-19");
        let names: Vec<_> = r.buckets[0].models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(names, ["claude-opus-5-5", "claude-sonnet-5", "gpt-6.1-sol"]);
    }

    #[test]
    fn report_query_rejects_invalid_dates_and_granularity() {
        let ok = ReportQuery::parse("2026-11-01", "2026-11-19", "day", None, None).unwrap();
        assert_eq!((ok.from.as_str(), ok.to.as_str()), ("2026-11-01", "2026-11-19"));
        assert_eq!(ReportQuery::parse("2026/11/01", "2026-11-19", "day", None, None).unwrap_err(), "日期格式错误：2026/11/01");
        assert_eq!(ReportQuery::parse("2026-11-01", "abc", "day", None, None).unwrap_err(), "日期格式错误：abc");
        assert!(ReportQuery::parse("2026-02-30", "2026-11-19", "day", None, None).is_err());
        assert!(ReportQuery::parse("2026-11-01", "2026-11-19", "week", None, None).is_err());
    }

    #[test]
    fn csv_field_quotes_special_chars() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn report_prices_one_hour_cache_write_at_1h_rate() {
        let (_t, s) = empty_stats();
        // opus-5-5：缓存写 5m 价 5、1h 价 8（美元/百万）
        put(&s, "2026-11-19", "claude", "claude-opus-5-5", Agg { n: 1, cw1h: 1_000_000, ..Default::default() });
        let r = s.report_at(&q("2026-11-19", "2026-11-19", Granularity::Day), "2026-11-19");
        assert_eq!(r.totals.cost_usd, 8.0);
        assert_eq!(r.totals.cache_write, 1_000_000);
    }

    #[test]
    fn granularity_parses_only_day_and_month() {
        assert_eq!(Granularity::parse("day"), Ok(Granularity::Day));
        assert_eq!(Granularity::parse("month"), Ok(Granularity::Month));
        assert!(Granularity::parse("week").is_err());
    }

    #[test]
    fn export_csv_writes_bom_and_header() {
        let (_t, s) = empty_stats();
        let today = day_of(SystemTime::now());
        put(&s, &today, "claude", "claude-opus-5-5", Agg { n: 2, e: 1, ms: 300, input: 1_000_000, out: 1_000_000, ..Default::default() });
        put(&s, &today, "codex", "gpt-6.1-sol", agg(1, 5, 6));
        let out = tempfile::tempdir().unwrap();
        let path = s.export_csv(&q(&today, &today, Granularity::Day), out.path()).unwrap();
        assert_eq!(path.file_name().unwrap().to_string_lossy(), format!("cc-bridge-usage-{today}_{today}.csv"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(
            lines[0],
            "period,route,model,requests,errors,avg_ms,input_uncached,cache_read,cache_write_5m,cache_write_1h,output,cost_usd"
        );
        let claude = lines.iter().find(|l| l.contains("claude-opus-5-5")).unwrap();
        assert!(claude.ends_with(",24.000000"), "{claude}");
        assert!(claude.contains(",2,1,150,"), "{claude}");
        let codex = lines.iter().find(|l| l.contains("gpt-6.1-sol")).unwrap();
        assert!(codex.ends_with(','), "未知价格留空：{codex}");
    }
}
