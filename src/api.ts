// 后端接口与类型；不在 Tauri 里运行时（截图预览）用 ?mock=<场景> 注入假数据。
import { mock } from './mock';

export type ProxyState =
  | { kind: 'starting' }
  | { kind: 'running' }
  | { kind: 'restarting'; attempt: number }
  | { kind: 'failed'; message: string }
  | { kind: 'portBusy'; pid: number | null; isClaudeCodex: boolean }
  | { kind: 'stopped' };

export type AuthState =
  | { kind: 'ok'; expiresAt: number }
  | { kind: 'expiredRefreshable' }
  | { kind: 'missing'; code: string };

export interface AppConfig {
  port: number;
  outbound_proxy: string | null;
  codex_home: string | null;
  claude_dir: string | null;
  autostart: boolean;
  takeover: boolean;
  selected_models: string[] | null;
  language: string | null;
}

export interface Status {
  proxy: ProxyState;
  port: number;
  auth: AuthState;
  authPath: string;
  authHint: string | null;
  networkHint: string | null;
  takeover: boolean;
  takenOver: boolean;
  settingsPath: string;
  settingsError: string | null;
  models: string[];
  selectedModels: string[];
  config: AppConfig;
  logsTail: string[];
  onboarded: boolean;
  devMode: boolean;
  lang: string;
  customApis: CustomApiDto[];
}

export interface EnvReport {
  codexCli: string | null;
  authPath: string;
  auth: AuthState;
  claudeSettings: string;
  settingsOk: boolean;
  settingsError: string | null;
  systemProxy: string | null;
  claudeCode: ClaudeCodeCheck;
}

/** 本机 Claude Code 版本检查（后端 lib.rs：ClaudeCodeCheck）。version 为 null 表示没找到 claude 命令 */
export interface ClaudeCodeCheck { version: string | null; min: string; ok: boolean }

/** 一次请求（后端 stats.rs：Activity） */
export interface Activity {
  id: string;
  at: number; // 完成时间，Unix 毫秒
  model: string;
  route: 'codex' | 'claude' | 'api';
  ok: boolean;
  httpStatus: number | null;
  ms: number;
  inputTokens: number | null;
  outputTokens: number | null;
  cacheReadTokens: number | null; // 缓存读 token；上游没给时为 null
  cacheWriteTokens: number | null; // 缓存写 token（5m + 1h）；上游没给时为 null
  error: string | null;
}

// ───────── 用量统计（后端 stats.rs：usage_report，camelCase） ─────────
/** 一组汇总：请求数、失败数、总耗时、四项 token 与等价 API 价（美元） */
export interface UsageTotals { count: number; errors: number; ms: number; input: number; cacheRead: number; cacheWrite: number; output: number; costUsd: number }
/** 某线路某模型在一个桶里的汇总；priced=false 表示没有价格，costUsd 恒为 0 */
export interface UsageModelRow { route: string; model: string; priced: boolean; totals: UsageTotals }
/** 一个时间桶：日桶 YYYY-MM-DD，月桶 YYYY-MM；只返回有数据的桶 */
export interface UsageBucket { key: string; byRoute: Record<string, UsageTotals>; models: UsageModelRow[] }
export interface UsageReport {
  from: string; to: string; granularity: 'day' | 'month'; days: number;
  totals: UsageTotals; byRoute: Record<string, UsageTotals>; buckets: UsageBucket[];
  unpricedModels: string[];
  routes: string[]; models: string[]; // 范围内出现过的全部线路 / 模型（不受过滤影响）
}

export interface Overview {
  recent: Activity[];
  today: { count: number; errors: number; avgMs: number };
  modelUse: Record<string, number>; // 近 7 天每个模型的调用次数
}

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type Listen = (ev: string, cb: (payload: Status) => void) => void;

const tauri = (window as any).__TAURI__;
const real = !!tauri?.core;

export const invoke: Invoke = real ? (cmd, args) => tauri.core.invoke(cmd, args) : mock.invoke;
export interface UpdateInfo {
  installing?: boolean;
  version: string;
  current: string;
  notes: string | null;
}

export const onEvent = <T,>(ev: string, cb: (payload: T) => void) => {
  if (real) void tauri.event.listen(ev, (e: { payload: T }) => cb(e.payload));
};

export const onStatus: Listen = real
  ? (ev, cb) => void tauri.event.listen(ev, (e: { payload: Status }) => cb(e.payload))
  : mock.listen;

// ───────── 自己的 Claude API（后端 custom_apis.rs / proxy/custom_api/verify.rs） ─────────
export interface CustomApiDto { id: string; baseUrl: string; keyMask: string; model: string; pickerName: string | null; verifiedAt: number | null }
export type StepId = 'auth' | 'format';
export type StepResult = { kind: 'pass' } | { kind: 'skip'; reason: string } | { kind: 'fail'; reason: string };
export interface VerifyReport { steps: [StepId, StepResult][]; authStyle: 'x_api_key' | 'bearer' | null; ok: boolean; transient: boolean }
export interface CustomApiStepEvent { id: string; step: StepId; result: StepResult }
