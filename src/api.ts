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
}

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
  error: string | null;
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
export type StepId = 'auth' | 'format' | 'tokenizer' | 'signature' | 'modelName';
export type StepResult = { kind: 'pass' } | { kind: 'skip'; reason: string } | { kind: 'fail'; reason: string };
export interface VerifyReport { steps: [StepId, StepResult][]; authStyle: 'x_api_key' | 'bearer' | null; ok: boolean; transient: boolean }
export interface CustomApiStepEvent { id: string; step: StepId; result: StepResult }
