// 仅用于浏览器里预览 / 截图：?mock=ok | portBusy | authMissing | onboard
import type { Status, EnvReport } from './api';

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
};

const t0 = Date.now();
const act = (i: number, model: string, route: 'codex' | 'claude', ok: boolean, ms: number, tin: number, tout: number, status = 200, error: string | null = null) =>
  ({ id: `r${i}`, at: t0 - i * 47000, model, route, ok, httpStatus: status, ms, inputTokens: tin, outputTokens: tout, error });
const activity = {
  recent: [
    act(0, 'gpt-6.1-sol', 'codex', true, 1840, 12840, 412),
    act(1, 'claude-opus-5-5', 'claude', true, 948, 9020, 233),
    act(2, 'gpt-6.1-sol', 'codex', false, 604, 11200, 0, 429, 'The usage limit has been reached'),
    act(3, 'gpt-6.1-sol-fast', 'codex', true, 1210, 8800, 1290),
    act(4, 'claude-opus-5-5', 'claude', true, 1057, 7410, 96),
  ],
  today: { count: 128, errors: 1, avgMs: 1900 },
  modelUse: { 'gpt-6.1-sol': 86, 'gpt-6.1-sol-fast': 31, 'gpt-6.1-terra': 12, 'gpt-6-astra': 2 },
};

export const mock = {
  invoke: async <T,>(cmd: string): Promise<T> => {
    const r: Record<string, unknown> = { get_status: status, detect_environment: env, test_outbound: 286, refresh_models: models, copy_logs: 3, app_version: '0.1.0', get_activity: activity, preview_start: null, finish_onboarding: null, set_models: null, set_takeover: null, save_config: null, get_update: new URLSearchParams(location.search).has('update') ? { version: '0.2.0', current: '0.1.0', notes: null } : null };
    return r[cmd] as T;
  },
  listen: () => {},
};
