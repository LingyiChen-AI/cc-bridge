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
  | { kind: 'missing'; reason: string };

export interface AppConfig {
  port: number;
  outbound_proxy: string | null;
  codex_home: string | null;
  claude_dir: string | null;
  autostart: boolean;
  takeover: boolean;
  selected_models: string[] | null;
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

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type Listen = (ev: string, cb: (payload: Status) => void) => void;

const tauri = (window as any).__TAURI__;
const real = !!tauri?.core;

export const invoke: Invoke = real ? (cmd, args) => tauri.core.invoke(cmd, args) : mock.invoke;
export const onStatus: Listen = real
  ? (ev, cb) => void tauri.event.listen(ev, (e: { payload: Status }) => cb(e.payload))
  : mock.listen;
