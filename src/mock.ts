// 仅用于浏览器里预览 / 截图：?mock=ok | portBusy | authMissing | onboard
import type { Status, EnvReport, StepId, StepResult } from './api';

const scene = new URLSearchParams(location.search).get('mock') ?? 'ok';
const models = ['gpt-6.1-sol', 'gpt-6.1-sol-fast', 'gpt-6.1-sol-ultrafast', 'gpt-6.1-terra', 'gpt-6.1-terra-fast', 'gpt-6-astra', 'gpt-6-astra-fast', 'gpt-6-sol', 'gpt-5.6-terra', 'gpt-5.6-mini', 'gpt-5.5', 'gpt-5.4'];
const now = Math.floor(Date.now() / 1000);

const base: Status = {
  proxy: { kind: 'running' },
  port: 18765,
  auth: { kind: 'ok', expiresAt: now + 9 * 86400 },
  authPath: 'C:\\Users\\me\\.codex\\auth.json',
  authHint: null,
  networkHint: null,
  takeover: true,
  takenOver: true,
  settingsPath: 'C:\\Users\\me\\.claude\\settings.json',
  settingsError: null,
  models,
  selectedModels: models.slice(0, 6),
  config: { port: 18765, outbound_proxy: 'http://127.0.0.1:7890', codex_home: null, claude_dir: null, autostart: true, takeover: true, selected_models: null, language: null },
  logsTail: [
    '[cc-bridge] 代理已启动 pid=27632 端口 18765',
    '{"level":"info","msg":"server listening","fields":{"port":18765}}',
    '{"level":"info","msg":"request_completed","fields":{"model":"gpt-6.1-sol","status":200,"ms":1840}}',
  ],
  onboarded: true,
  devMode: new URLSearchParams(location.search).has('dev'),
  lang: new URLSearchParams(location.search).get('lang') ?? 'zh',
  customApis: [
    { id: 'c1', baseUrl: 'https://api.anthropic.com', keyMask: 'sk-ant-…a9Zq', model: 'claude-opus-5-5', pickerName: 'API claude-opus-5-5', verifiedAt: now - 3600 },
    { id: 'c2', baseUrl: 'https://dashscope.aliyuncs.com/apps/anthropic', keyMask: 'sk-…41fd', model: 'claude-sonnet-5', pickerName: 'API claude-sonnet-5', verifiedAt: null },
  ],
};

const scenes: Record<string, Partial<Status>> = {
  ok: {},
  portBusy: { proxy: { kind: 'portBusy', pid: 73876, isClaudeCodex: true }, takenOver: false, models: [] },
  authMissing: {
    auth: { kind: 'missing', code: 'notFound' },
    authHint: 'No Codex credentials. Log in with the Codex CLI (`codex login`) to create ~/.codex/auth.json',
    networkHint: null,
  },
  onboard: { onboarded: false, takenOver: false, proxy: { kind: 'stopped' } },
};

const status: Status = { ...base, ...(scenes[scene] ?? {}) };
const env: EnvReport = {
  codexCli: 'codex-cli 0.167.0',
  authPath: status.authPath,
  auth: status.auth,
  claudeSettings: status.settingsPath,
  settingsOk: true,
  settingsError: null,
  systemProxy: 'http://127.0.0.1:7890',
  // ?ccold 预览 Claude Code 版本过旧的提示
  claudeCode: new URLSearchParams(location.search).has('ccold') ? { version: '2.1.220', min: '2.1.243', ok: false } : { version: '2.1.329', min: '2.1.243', ok: true },
};

const t0 = Date.now();
const act = (i: number, model: string, route: 'codex' | 'claude' | 'api', ok: boolean, ms: number, tin: number, tout: number, status = 200, error: string | null = null) =>
  ({ id: `r${i}`, at: t0 - i * 47000, model, route, ok, httpStatus: status, ms, inputTokens: tin, outputTokens: tout, cacheReadTokens: null, cacheWriteTokens: null, error });
const activity = {
  recent: [
    act(0, 'gpt-6.1-sol', 'codex', true, 1840, 12840, 412),
    act(1, 'claude-opus-5-5', 'claude', true, 948, 9020, 233),
    act(2, 'gpt-6.1-sol', 'codex', false, 604, 11200, 0, 429, 'The usage limit has been reached'),
    act(3, 'gpt-6.1-sol-fast', 'codex', true, 1210, 8800, 1290),
    act(4, 'claude-opus-5-5', 'claude', true, 1057, 7410, 96),
    act(5, 'claude-opus-5-5', 'api', true, 1320, 5300, 180),
  ],
  today: { count: 128, errors: 1, avgMs: 1900 },
  modelUse: { 'gpt-6.1-sol': 86, 'gpt-6.1-sol-fast': 31, 'gpt-6.1-terra': 12, 'gpt-6-astra': 2 },
};

// ───────── 用量统计 mock：近 90 天三条线路，按日期字符串做种子，结果稳定 ─────────
const USAGE_MODELS: [route: string, model: string, price: number[] | null][] = [
  ['claude', 'claude-opus-5-5', [4, 20, 0.2, 5]], // 美元 / 百万 token：输入 / 输出 / 缓存读 / 缓存写 5m
  ['claude', 'claude-sonnet-5-5', [2, 10, 0.1, 2.5]],
  ['codex', 'gpt-6.1-sol', null], // Codex 订阅：未知价格
  ['api', 'api-claude-opus-5-5', null], // 自己的 API：不计价
];
const ymdOf = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
const addDays = (s: string, n: number) => { const [y, m, d] = s.split('-').map(Number); return ymdOf(new Date(y, m - 1, d + n)); };
/** 小型确定性随机数（mulberry32），种子取日期 + 模型名的哈希 */
function rng(seed: string) {
  let h = 2166136261;
  for (const c of seed) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return () => { h = (h + 0x6d2b79f5) | 0; let x = Math.imul(h ^ (h >>> 15), 1 | h); x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x; return ((x ^ (x >>> 14)) >>> 0) / 4294967296; };
}
type Tot = { count: number; errors: number; ms: number; input: number; cacheRead: number; cacheWrite: number; output: number; costUsd: number };
const zero = (): Tot => ({ count: 0, errors: 0, ms: 0, input: 0, cacheRead: 0, cacheWrite: 0, output: 0, costUsd: 0 });
const addTot = (a: Tot, b: Tot) => { for (const k of Object.keys(a) as (keyof Tot)[]) a[k] += b[k]; };
function dayUsage(day: string, route: string, model: string, price: number[] | null): Tot | null {
  const r = rng(`${day}|${model}`);
  const weight = route === 'codex' ? 1 : route === 'claude' ? (model.includes('opus') ? 0.6 : 0.35) : 0.15;
  if (r() > 0.25 + weight * 0.7) return null; // 有的天没用这个模型
  const count = Math.max(1, Math.round(r() * 120 * weight));
  const input = Math.round(count * (800 + r() * 4000));
  const cacheRead = Math.round(count * (5000 + r() * 30000));
  const cacheWrite = Math.round(count * r() * 3000);
  const output = Math.round(count * (200 + r() * 1500));
  const costUsd = price ? (input * price[0] + output * price[1] + cacheRead * price[2] + cacheWrite * price[3]) / 1e6 : 0;
  return { count, errors: r() < 0.2 ? 1 : 0, ms: count * Math.round(900 + r() * 2000), input, cacheRead, cacheWrite, output, costUsd };
}
function usageReport(args: Record<string, unknown> = {}) {
  const re = /^\d{4}-\d{2}-\d{2}$/;
  let from = String(args.from ?? ''), to = String(args.to ?? '');
  if (!re.test(from) || !re.test(to)) throw `日期格式错误：${re.test(from) ? to : from}`;
  const gran = String(args.granularity);
  if (gran !== 'day' && gran !== 'month') throw `粒度错误：${gran}`;
  const route = (args.route as string | null) ?? null, model = (args.model as string | null) ?? null;
  const today = ymdOf(new Date());
  if (from > to) [from, to] = [to, from];
  if (to > today) to = today;
  if (from < addDays(today, -399)) from = addDays(today, -399);
  const totals = zero(), byRoute: Record<string, Tot> = {};
  const buckets: { key: string; byRoute: Record<string, Tot>; models: { route: string; model: string; priced: boolean; totals: Tot }[] }[] = [];
  const unpriced = new Set<string>(), routes = new Set<string>(), models = new Set<string>();
  let days = 0;
  for (let d = from; d <= to; d = addDays(d, 1)) {
    days++;
    if (d < addDays(today, -89)) continue; // 只生成近 90 天
    const key = gran === 'month' ? d.slice(0, 7) : d;
    for (const [rt, m, price] of USAGE_MODELS) {
      const u = dayUsage(d, rt, m, price);
      if (!u) continue;
      routes.add(rt); models.add(m);
      if ((route && route !== rt) || (model && model !== m)) continue;
      if (!price) unpriced.add(m);
      addTot(totals, u);
      addTot((byRoute[rt] ??= zero()), u);
      let b = buckets[buckets.length - 1];
      if (!b || b.key !== key) buckets.push((b = { key, byRoute: {}, models: [] }));
      addTot((b.byRoute[rt] ??= zero()), u);
      const row = b.models.find((x) => x.route === rt && x.model === m);
      if (row) addTot(row.totals, u); else b.models.push({ route: rt, model: m, priced: !!price, totals: { ...u } });
    }
  }
  const tok = (x: Tot) => x.input + x.cacheRead + x.cacheWrite + x.output;
  for (const b of buckets) b.models.sort((x, y) => y.totals.costUsd - x.totals.costUsd || tok(y.totals) - tok(x.totals) || x.model.localeCompare(y.model));
  return { from, to, granularity: gran, days, totals, byRoute, buckets, unpricedModels: [...unpriced].sort(), routes: [...routes].sort(), models: [...models].sort() };
}

// 预览校验弹窗：?mock&capi=progress 打开弹窗并停在校验中途；capi=fail 显示失败态
const STEPS: StepId[] = ['auth', 'format'];
export const mockStep = (n: number, fail: boolean): [StepId, StepResult][] =>
  STEPS.slice(0, n).map((id, i): [StepId, StepResult] => [id, fail && i === n - 1 ? { kind: 'fail', reason: '不是 Claude（Anthropic Messages）格式的接口' } : { kind: 'pass' }]);

export const mock = {
  invoke: async <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    if (cmd === 'save_custom_api' || cmd === 'reverify_custom_api') {
      const fail = String((args?.input as { baseUrl?: string } | undefined)?.baseUrl ?? '').includes('openai');
      return { steps: mockStep(2, fail), authStyle: 'x_api_key', ok: !fail } as T;
    }
    if (cmd === 'usage_report') return usageReport(args) as T;
    if (cmd === 'export_usage_csv') { usageReport(args); return `C:\\Users\\me\\Downloads\\cc-bridge-usage-${String(args?.from)}-${String(args?.to)}.csv` as T; }
    const r: Record<string, unknown> = { get_status: status, detect_environment: env, claude_code_check: env.claudeCode, test_outbound: 286, refresh_models: models, copy_logs: 3, app_version: '0.1.0', get_activity: activity, preview_start: null, finish_onboarding: null, set_models: null, set_takeover: null, save_config: null, get_update: new URLSearchParams(location.search).has('update') ? { version: '0.2.0', current: '0.1.0', notes: null } : null };
    return r[cmd] as T;
  },
  listen: () => {},
};
