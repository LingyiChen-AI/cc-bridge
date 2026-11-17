// 界面中英文。后端返回错误码 / 原始详情，这里负责翻译。
export type Lang = 'zh' | 'en';

const zh = {
  appTagline: '把 Codex 模型接入 Claude Code',
  overallOk: '运行中', overallWarn: '需要留意', overallErr: '有问题',
  // 代理
  proxyRunning: '运行中', proxyStarting: '启动中…', proxyRestarting: '重启中（第 {n} 次）', proxyRestartingMeta: '进程退出或健康检查失败',
  proxyFailed: '已停止', proxyFailedMeta: '1 分钟内反复崩溃，已停止自动重启',
  portBusy: '端口 {port} 被占用', portBusyOurs: '已有一个 claude-codex 在运行（pid {pid}）', portBusyOther: '被其它程序占用（pid {pid}）', unknown: '未知',
  proxyStopped: '未启动',
  // 登录
  auth401: '登录无效（401）', auth401Meta: '代理读不到可用的 Codex 令牌',
  authOk: '已登录', authOkMeta: '令牌到期 {time}，代理会自动续期', authExpiresAt: '到期 {time}',
  authRefresh: '令牌待续期', authRefreshMeta: '下次请求时代理会自动续期，无需操作',
  authMissing: '未登录', needLogin: '需要登录',
  authCode_notFound: '未找到 Codex 登录文件，请运行 codex login',
  authCode_invalidJson: 'Codex 登录文件不是合法 JSON',
  authCode_noToken: '登录文件里没有 ChatGPT 令牌，请运行 codex login',
  authCode_expired: '令牌已过期且无法续期，请运行 codex login',
  // 接管
  settingsErrTitle: '无法修改配置',
  settingsErr_invalid: 'settings.json 不是合法 JSON，已停止修改：{detail}',
  settingsErr_io: '读写 settings.json 失败：{detail}',
  taken: '已接管', takenMeta: '新开的 Claude Code 窗口可选 Codex 模型', takenSub: '新开窗口生效',
  waiting: '等待接管', waitingMeta: '代理就绪后自动写入',
  notTaken: '未接管', notTakenMeta: 'Claude Code 只使用 Claude 模型',
  takeoverTitle: '接管 Claude Code', takeoverSub: '新开窗口可在 /model 里选 Codex 模型',
  // 告警
  portBusyHintOurs: '可能是以前手动启动的代理，由本客户端接管更稳定。', portBusyHintOther: '请在设置里换一个端口。', killAndTake: '结束它并接管',
  failedTitle: '代理反复崩溃', failedBody: '1 分钟内反复退出，已停止自动重启。', retry: '重试',
  loginTitle: 'Codex 登录不可用',
  netTitle: '出口网络不可用', netBody: '代理连不上 OpenAI，检查出口代理是否开启。',
  devTitle: '开发模式 · 不影响全局 Claude Code', devBody: '用这条命令单独开窗口测试：', copy: '复制',
  // 区块
  secStatus: '状态', secProxy: '代理', restartProxy: '重启代理',
  secModels: '模型 · 已选 {n} / {total}', resetDefault: '恢复默认', selectAll: '全选', selectNone: '清空', refresh: '刷新',
  latest: '最新', modelsEmpty: '代理就绪后显示', expandRest: '展开其余 {n} 个', collapse: '收起',
  modelsNote: '只有勾选的模型会出现在 Claude Code 的 /model 里', modelsNoteDefault: '（默认最新 {n} 个）',
  logs: '日志', settings: '设置', noLogs: '暂无日志', copyAll: '复制全部',
  foot: '改动只对新开的 Claude Code 窗口生效 · 关闭窗口会缩到托盘',
  // 设置
  port: '端口', outbound: '出口代理', outboundPh: '留空 = 直连', codexDir: 'Codex 目录', claudeDir: 'Claude 目录', dirDefault: '默认 {dir}',
  autostart: '开机自启', language: '界面语言', langAuto: '跟随系统', cancel: '取消', saveApply: '保存并应用',
  // 引导
  setupBadge: '首次设置 {n}/{total}', next: '下一步', back: '上一步', done: '完成',
  welcome: '欢迎使用 CC Bridge', detecting: '正在检测本机环境…',
  envTitle: '检查环境', envLead: '需要 Codex CLI 已登录 ChatGPT 账号，并能修改 Claude Code 的配置。',
  cliMissing: '未安装。安装命令：', chatgptLogin: 'ChatGPT 登录', openLogin: '打开终端运行 codex login', recheck: '重新检测',
  claudeConfig: 'Claude Code 配置',
  netStepTitle: '出口网络', netStepLead: '代理需要访问 OpenAI。国内网络一般要填本机代理地址，例如',
  testConn: '测试连通', testing: '正在连接 chatgpt.com…', sysProxyFound: '已检测到系统代理 {proxy}', sysProxyNone: '未检测到系统代理',
  connOk: '连通，耗时 {ms} ms',
  conn_timeout: '连接超时', conn_connect: '无法连接（检查代理是否开启）', conn_badProxy: '代理地址无效：{detail}', conn_other: '连接失败：{detail}',
  runTitle: '运行方式', runLead: '代理只监听本机，端口被占用时可以换一个。', autostartSub: '开机后只在托盘运行',
  startTitle: '启动代理', startLead: '代理就绪后会列出可用的 Codex 模型。', availableModels: '可用模型', waitProxy: '等待代理就绪',
  takeoverWizardSub: '写入 settings.json，退出客户端时自动恢复',
  // 提示
  toastRestart: '正在重启代理', toastKill: '正在接管端口', toastLogin: '已打开终端，登录完成后会自动检测',
  toastRefreshed: '已刷新，共 {n} 个模型', toastCopiedCmd: '已复制命令', toastCopiedLogs: '已复制 {n} 行日志', toastSaved: '已保存', toastDone: '设置完成',
  err_noCli: '未安装 Codex CLI：npm i -g @openai/codex', err_terminal: '打开终端失败：{detail}',
};

const en: typeof zh = {
  appTagline: 'Use Codex models in Claude Code',
  overallOk: 'Running', overallWarn: 'Attention', overallErr: 'Problem',
  proxyRunning: 'Running', proxyStarting: 'Starting…', proxyRestarting: 'Restarting (#{n})', proxyRestartingMeta: 'Process exited or health check failed',
  proxyFailed: 'Stopped', proxyFailedMeta: 'Crashed repeatedly within a minute; auto-restart stopped',
  portBusy: 'Port {port} in use', portBusyOurs: 'Another claude-codex is running (pid {pid})', portBusyOther: 'Used by another program (pid {pid})', unknown: 'unknown',
  proxyStopped: 'Not started',
  auth401: 'Invalid login (401)', auth401Meta: 'The proxy cannot find a usable Codex token',
  authOk: 'Signed in', authOkMeta: 'Token expires {time}; the proxy refreshes it automatically', authExpiresAt: 'Expires {time}',
  authRefresh: 'Token pending refresh', authRefreshMeta: 'The proxy refreshes it on the next request; nothing to do',
  authMissing: 'Not signed in', needLogin: 'Sign-in required',
  authCode_notFound: 'Codex login file not found. Run codex login',
  authCode_invalidJson: 'Codex login file is not valid JSON',
  authCode_noToken: 'No ChatGPT token in the login file. Run codex login',
  authCode_expired: 'Token expired and cannot be refreshed. Run codex login',
  settingsErrTitle: 'Cannot edit settings',
  settingsErr_invalid: 'settings.json is not valid JSON; no changes were made: {detail}',
  settingsErr_io: 'Failed to read or write settings.json: {detail}',
  taken: 'Connected', takenMeta: 'New Claude Code windows can pick Codex models', takenSub: 'Applies to new windows',
  waiting: 'Waiting', waitingMeta: 'Will be written once the proxy is ready',
  notTaken: 'Not connected', notTakenMeta: 'Claude Code uses Claude models only',
  takeoverTitle: 'Connect Claude Code', takeoverSub: 'New windows can pick Codex models in /model',
  portBusyHintOurs: 'Probably a proxy you started manually earlier. Letting this app manage it is more reliable.', portBusyHintOther: 'Pick another port in Settings.', killAndTake: 'Stop it and take over',
  failedTitle: 'Proxy keeps crashing', failedBody: 'It exited repeatedly within a minute; auto-restart stopped.', retry: 'Retry',
  loginTitle: 'Codex login unavailable',
  netTitle: 'Outbound network unavailable', netBody: 'The proxy cannot reach OpenAI. Check that your outbound proxy is running.',
  devTitle: 'Dev mode · global Claude Code is not affected', devBody: 'Open a separate window to test with:', copy: 'Copy',
  secStatus: 'Status', secProxy: 'Proxy', restartProxy: 'Restart proxy',
  secModels: 'Models · {n} / {total} selected', resetDefault: 'Reset', selectAll: 'All', selectNone: 'None', refresh: 'Refresh',
  latest: 'Latest', modelsEmpty: 'Shown once the proxy is ready', expandRest: 'Show {n} more', collapse: 'Show less',
  modelsNote: 'Only checked models appear in Claude Code’s /model', modelsNoteDefault: ' (default: newest {n})',
  logs: 'Logs', settings: 'Settings', noLogs: 'No logs yet', copyAll: 'Copy all',
  foot: 'Changes apply to new Claude Code windows · Closing this window keeps it in the tray',
  port: 'Port', outbound: 'Outbound proxy', outboundPh: 'Empty = direct', codexDir: 'Codex folder', claudeDir: 'Claude folder', dirDefault: 'Default {dir}',
  autostart: 'Launch at login', language: 'Language', langAuto: 'System', cancel: 'Cancel', saveApply: 'Save & apply',
  setupBadge: 'Setup {n}/{total}', next: 'Next', back: 'Back', done: 'Done',
  welcome: 'Welcome to CC Bridge', detecting: 'Checking your environment…',
  envTitle: 'Check environment', envLead: 'Codex CLI must be signed in to ChatGPT, and Claude Code settings must be writable.',
  cliMissing: 'Not installed. Install with:', chatgptLogin: 'ChatGPT sign-in', openLogin: 'Run codex login in a terminal', recheck: 'Check again',
  claudeConfig: 'Claude Code settings',
  netStepTitle: 'Outbound network', netStepLead: 'The proxy needs to reach OpenAI. If you use a local proxy, enter it here, e.g.',
  testConn: 'Test', testing: 'Connecting to chatgpt.com…', sysProxyFound: 'System proxy detected: {proxy}', sysProxyNone: 'No system proxy detected',
  connOk: 'Reachable in {ms} ms',
  conn_timeout: 'Connection timed out', conn_connect: 'Cannot connect (is the proxy running?)', conn_badProxy: 'Invalid proxy address: {detail}', conn_other: 'Connection failed: {detail}',
  runTitle: 'Startup', runLead: 'The proxy listens on this machine only. Change the port if it is taken.', autostartSub: 'Runs in the tray after login',
  startTitle: 'Start proxy', startLead: 'Available Codex models appear once the proxy is ready.', availableModels: 'Models', waitProxy: 'Waiting for the proxy',
  takeoverWizardSub: 'Writes settings.json; restored automatically when you quit',
  toastRestart: 'Restarting proxy', toastKill: 'Taking over the port', toastLogin: 'Terminal opened; sign-in is detected automatically',
  toastRefreshed: 'Refreshed, {n} models', toastCopiedCmd: 'Command copied', toastCopiedLogs: 'Copied {n} log lines', toastSaved: 'Saved', toastDone: 'Setup complete',
  err_noCli: 'Codex CLI is not installed: npm i -g @openai/codex', err_terminal: 'Failed to open a terminal: {detail}',
};

export type Key = keyof typeof zh;
const dicts: Record<Lang, typeof zh> = { zh, en };
let current: Lang = navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en';

export function setLang(l: string | undefined | null) {
  if (l === 'zh' || l === 'en') {
    current = l;
    document.documentElement.lang = l === 'zh' ? 'zh-CN' : 'en';
  }
}

export function t(key: Key, vars: Record<string, string | number> = {}): string {
  return dicts[current][key].replace(/\{(\w+)\}/g, (_, k) => String(vars[k] ?? ''));
}

/** 后端返回的 "code: detail" 形式错误 → 翻译文本 */
export function trError(prefix: 'conn_' | 'err_' | 'settingsErr_', raw: string): string {
  const s = String(raw).replace(/^Error:\s*/, '');
  const i = s.indexOf(':');
  const code = (i >= 0 ? s.slice(0, i) : s).trim();
  const detail = i >= 0 ? s.slice(i + 1).trim() : '';
  const key = `${prefix}${code}` as Key;
  return key in zh ? t(key, { detail }) : s;
}
