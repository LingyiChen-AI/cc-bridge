// 仅用于浏览器里预览 / 截图：?mock=ok | portBusy | authMissing | onboard
import type { Status, EnvReport } from './api';

const scene = new URLSearchParams(location.search).get('mock') ?? 'ok';
const models = ['gpt-6.1-sol', 'gpt-6.1-terra', 'gpt-6.1-terra-fast', 'gpt-6-astra', 'gpt-6-sol', 'gpt-5.6-terra', 'gpt-5.6-mini'];
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
  config: { port: 18765, outbound_proxy: 'http://127.0.0.1:7890', codex_home: null, claude_dir: null, autostart: true, takeover: true },
  upstreamVersion: 'v0.8.0',
  logsTail: [
    '[codex-bridge] 代理已启动 pid=27632 端口 18765',
    '{"level":"info","msg":"server listening","fields":{"port":18765}}',
    '{"level":"info","msg":"request_completed","fields":{"model":"gpt-6.1-sol","status":200,"ms":1840}}',
  ],
  onboarded: true,
  devMode: new URLSearchParams(location.search).has('dev'),
};

const scenes: Record<string, Partial<Status>> = {
  ok: {},
  portBusy: { proxy: { kind: 'portBusy', pid: 73876, isClaudeCodex: true }, takenOver: false, models: [] },
  authMissing: {
    auth: { kind: 'missing', reason: '未找到 Codex 登录文件，请运行 codex login' },
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

export const mock = {
  invoke: async <T,>(cmd: string): Promise<T> => {
    const r: Record<string, unknown> = { get_status: status, detect_environment: env, test_outbound: 286, refresh_models: models, copy_logs: 3 };
    return r[cmd] as T;
  },
  listen: () => {},
};
