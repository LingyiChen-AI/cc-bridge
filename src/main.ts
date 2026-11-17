import { invoke, onStatus, type AppConfig, type AuthState, type EnvReport, type ProxyState, type Status } from './api';
import { icon } from './icons';

type Tone = 'ok' | 'warn' | 'err';
const app = document.getElementById('app')!;
const esc = (s: unknown) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);

let status: Status | null = null;

// ───────── 文案 ─────────
function proxyInfo(p: ProxyState, port: number): { tone: Tone; value: string; meta: string } {
  switch (p.kind) {
    case 'running': return { tone: 'ok', value: '运行中', meta: `127.0.0.1:${port}` };
    case 'starting': return { tone: 'warn', value: '启动中…', meta: `127.0.0.1:${port}` };
    case 'restarting': return { tone: 'warn', value: `重启中（第 ${p.attempt} 次）`, meta: '进程退出或健康检查失败' };
    case 'failed': return { tone: 'err', value: '已停止', meta: '1 分钟内反复崩溃，已停止自动重启' };
    case 'portBusy': return { tone: 'err', value: `端口 ${port} 被占用`, meta: p.isClaudeCodex ? `已有一个 claude-codex 在运行（pid ${p.pid}）` : `被其它程序占用（pid ${p.pid ?? '未知'}）` };
    case 'stopped': return { tone: 'warn', value: '未启动', meta: '' };
  }
}

function fmtTime(sec: number) {
  const d = new Date(sec * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function authInfo(a: AuthState, hint: string | null): { tone: Tone; value: string; meta: string } {
  if (hint) return { tone: 'err', value: '登录无效（401）', meta: '代理读不到可用的 Codex 令牌' };
  switch (a.kind) {
    case 'ok': return { tone: 'ok', value: '已登录', meta: `令牌到期 ${fmtTime(a.expiresAt)}，代理会自动续期` };
    case 'expiredRefreshable': return { tone: 'warn', value: '令牌待续期', meta: '下次请求时代理会自动续期，无需操作' };
    case 'missing': return { tone: 'err', value: '未登录', meta: a.reason };
  }
}

function takeoverInfo(s: Status): { tone: Tone; value: string; meta: string } {
  if (s.settingsError) return { tone: 'err', value: '无法修改配置', meta: s.settingsError };
  if (s.takenOver) return { tone: 'ok', value: '已接管', meta: '新开的 Claude Code 窗口可选 Codex 模型' };
  if (s.takeover) return { tone: 'warn', value: '等待接管', meta: '代理就绪后自动写入' };
  return { tone: 'warn', value: '未接管', meta: 'Claude Code 只使用 Claude 模型' };
}

// ───────── 主界面 ─────────
function renderMain(s: Status) {
  const p = proxyInfo(s.proxy, s.port);
  const a = authInfo(s.auth, s.authHint);
  const t = takeoverInfo(s);
  const tones = [p.tone, a.tone, t.tone];
  const overall: Tone = tones.includes('err') ? 'err' : tones.includes('warn') ? 'warn' : 'ok';
  const overallText = { ok: '一切正常', warn: '需要留意', err: '有问题' }[overall];

  const alerts: string[] = [];
  if (s.proxy.kind === 'portBusy') {
    alerts.push(alert('err', '端口被占用', s.proxy.isClaudeCodex ? '可能是以前手动启动的代理，由本客户端接管更稳定。' : '请在设置里换一个端口。',
      s.proxy.isClaudeCodex ? `<button class="btn danger" data-act="kill-port">结束它并接管</button>` : ''));
  }
  if (s.proxy.kind === 'failed') alerts.push(alert('err', '代理反复崩溃', `<code>${esc(s.proxy.message)}</code>`, `<button class="btn" data-act="restart">重试</button>`));
  if (s.authHint || s.auth.kind === 'missing') {
    alerts.push(alert('err', 'Codex 登录不可用', s.authHint ? `<code>${esc(s.authHint)}</code>` : esc((s.auth as any).reason), `<button class="btn" data-act="login">运行 codex login</button>`));
  }
  if (s.devMode) {
    const cmd = `claude --settings "${s.settingsPath}"`;
    alerts.push(alert('info', '开发模式 · 只写沙盒配置，不影响全局 Claude Code',
      `用这条命令单独开一个窗口测试：<br><code>${esc(cmd)}</code>`,
      `<button class="btn" data-act="copy-dev-cmd" data-cmd="${esc(cmd)}">${icon.copy()} 复制</button>`));
  }
  if (s.networkHint) alerts.push(alert('warn', '出口网络不可用', `代理连不上 OpenAI，检查出口代理是否开启。<br><code>${esc(s.networkHint)}</code>`, ''));

  const top = `
    <div class="top">
      <div class="brand">${icon.logo()}</div>
      <div class="grow"><h1>Codex Bridge</h1><div class="sub">${s.devMode ? '开发模式 · 端口 ' + s.port : '把 Codex 模型接入 Claude Code'}</div></div>
      <span class="pill ${overall}"><span class="dot ${overall === 'ok' ? '' : 'pulse'}"></span>${overallText}</span>
    </div>`;

  const firstIssue = [a, p, t].find((x) => x.tone !== 'ok');
  const heroMeta = firstIssue ? firstIssue.meta : `Claude Code → 127.0.0.1:${s.port} → ChatGPT 订阅`;
  const live = s.proxy.kind === 'running';
  const cards = `
    <section class="hero">
      <div class="flow">
        ${node(a.tone, icon.key(20), 'Codex 登录', a.value)}
        ${link(a.tone === 'ok' && p.tone === 'ok', live)}
        ${node(p.tone, icon.server(20), '代理', p.value)}
        ${link(p.tone === 'ok' && s.takenOver, live && s.takenOver)}
        ${node(t.tone, icon.terminal(20), 'Claude Code', t.value)}
      </div>
      <div class="hero-foot">
        <span class="hero-meta" title="${esc(heroMeta)}">${esc(heroMeta)}</span>
        <button class="btn icon ghost" data-act="restart" title="重启代理" aria-label="重启代理">${icon.refresh()}</button>
      </div>
    </section>
    <div class="card">
      <div class="body"><div class="value">接管 Claude Code</div><div class="meta">${esc(t.meta)}</div></div>
      <label class="switch" title="接管 Claude Code"><input type="checkbox" data-act="takeover" ${s.takeover ? 'checked' : ''} aria-label="接管 Claude Code"><span></span></label>
    </div>`;

  const models = `
    <section class="section">
      <header>可用 Codex 模型 <span class="count">${s.models.length}</span><span class="grow"></span>
        <button class="btn icon ghost" data-act="refresh-models" title="刷新" aria-label="刷新模型">${icon.refresh()}</button></header>
      <div class="content">${s.models.length ? `<div class="chips">${s.models.map((m) => `<span class="chip">${esc(m)}</span>`).join('')}</div>` : `<div class="empty">代理就绪后显示</div>`}</div>
    </section>`;

  const logs = `
    <details class="section" id="logs-sec" ${logsOpen ? 'open' : ''}>
      <summary>${icon.log()} 日志<span class="count" style="color:var(--text-3);font-weight:500">${s.logsTail.length}</span>
        <span class="chev">${icon.chevron()}</span></summary>
      <div class="content">
        <div class="logs" id="logs">${s.logsTail.map(logLine).join('\n') || '暂无日志'}</div>
        <div class="row between" style="margin-top:8px"><span class="empty">上游 claude-codex ${esc(s.upstreamVersion)}</span>
        <button class="btn" data-act="copy-logs">${icon.copy()} 复制全部</button></div>
      </div>
    </details>`;

  app.innerHTML = `${top}${alerts.join('')}${cards}${models}<div id="settings-slot"></div>${logs}
    <footer class="foot">改动只对新开的 Claude Code 窗口生效 · 关闭窗口会缩到托盘</footer>`;
  mountSettings(s.config);
  const box = document.getElementById('logs');
  if (box) box.scrollTop = box.scrollHeight;
  document.getElementById('logs-sec')?.addEventListener('toggle', (e) => (logsOpen = (e.target as HTMLDetailsElement).open));
}
let logsOpen = false;

const card = (tone: Tone, ic: string, title: string, value: string, meta: string, action: string) => `
  <div class="card"><div class="ic ${tone}">${ic}</div>
    <div class="body"><div class="title">${title}</div><div class="value">${esc(value)}</div><div class="meta" title="${esc(meta)}">${esc(meta)}</div></div>
    ${action}</div>`;

const node = (tone: Tone, ic: string, label: string, value: string) => `
  <div class="node ${tone}"><div class="node-ic">${ic}<span class="node-dot"></span></div>
    <div class="node-label">${label}</div><div class="node-value">${esc(value)}</div></div>`;

const link = (ok: boolean, flowing: boolean) => `<div class="link ${ok ? 'ok' : ''} ${flowing ? 'flowing' : ''}"><span></span></div>`;

const alert = (tone: Tone | 'info', title: string, body: string, action: string) => `
  <div class="alert ${tone}">${icon.alert()}<div class="msg"><b>${title}</b>${body}</div>${action}</div>`;

function logLine(l: string) {
  const cls = /"level":"(error)"|失败|error/i.test(l) ? 'e' : /"level":"warn"|占用|超时|重启/.test(l) ? 'w' : '';
  return cls ? `<span class="${cls}">${esc(l)}</span>` : esc(l);
}

// 设置区：自己维护表单状态，不被状态推送覆盖
let settingsDraft: AppConfig | null = null;
let settingsOpen = false;
function mountSettings(cfg: AppConfig) {
  const slot = document.getElementById('settings-slot')!;
  const d = settingsDraft ?? { ...cfg };
  const dirty = settingsDraft && JSON.stringify(settingsDraft) !== JSON.stringify(cfg);
  slot.outerHTML = `
    <details class="section" id="settings" ${settingsOpen ? 'open' : ''}>
      <summary>${icon.settings()} 设置<span class="chev">${icon.chevron()}</span></summary>
      <div class="content">
        <div class="row">
          <div class="field" style="width:110px"><label for="f-port">端口</label><input id="f-port" class="input mono" inputmode="numeric" value="${d.port}"></div>
          <div class="field" style="flex:1"><label for="f-proxy">出口代理</label><input id="f-proxy" class="input mono" placeholder="留空 = 直连" value="${esc(d.outbound_proxy ?? '')}"></div>
        </div>
        <div class="field"><label for="f-codex">Codex 目录</label><input id="f-codex" class="input mono" placeholder="默认 ~/.codex" value="${esc(d.codex_home ?? '')}"></div>
        <div class="field"><label for="f-claude">Claude 目录</label><input id="f-claude" class="input mono" placeholder="默认 ~/.claude" value="${esc(d.claude_dir ?? '')}"></div>
        <div class="toggle-row"><span>开机自启</span><label class="switch"><input type="checkbox" id="f-auto" ${d.autostart ? 'checked' : ''} aria-label="开机自启"><span></span></label></div>
        <div class="row" style="justify-content:flex-end;gap:8px;${dirty ? '' : 'display:none'}" id="settings-actions">
          <button class="btn ghost" data-act="settings-reset">取消</button>
          <button class="btn primary" data-act="settings-save">${icon.check()} 保存并应用</button>
        </div>
      </div>
    </details>`;
  const el = document.getElementById('settings') as HTMLDetailsElement;
  el.addEventListener('toggle', () => (settingsOpen = el.open));
  el.addEventListener('input', () => {
    settingsDraft = readForm(cfg);
    const changed = JSON.stringify(settingsDraft) !== JSON.stringify(cfg);
    (document.getElementById('settings-actions') as HTMLElement).style.display = changed ? '' : 'none';
  });
}

function readForm(base: AppConfig): AppConfig {
  const v = (id: string) => (document.getElementById(id) as HTMLInputElement).value.trim();
  const port = Number(v('f-port'));
  return {
    ...base,
    port: Number.isInteger(port) && port > 0 && port < 65536 ? port : base.port,
    outbound_proxy: v('f-proxy') || null,
    codex_home: v('f-codex') || null,
    claude_dir: v('f-claude') || null,
    autostart: (document.getElementById('f-auto') as HTMLInputElement).checked,
  };
}

// ───────── 首次运行引导 ─────────
const wiz = { step: 0, env: null as EnvReport | null, cfg: null as AppConfig | null, test: null as null | { ok: boolean; text: string } | 'loading', started: false };
const STEPS = 4;

async function loadEnv() {
  wiz.env = await invoke<EnvReport>('detect_environment');
  if (!wiz.cfg) {
    wiz.cfg = { ...(status?.config ?? { port: 18765, outbound_proxy: null, codex_home: null, claude_dir: null, autostart: true, takeover: true }) };
    if (!wiz.cfg.outbound_proxy && wiz.env.systemProxy) wiz.cfg.outbound_proxy = wiz.env.systemProxy;
  }
  render();
}

function checkRow(tone: Tone, title: string, desc: string, act = '') {
  const ic = tone === 'ok' ? icon.check(14) : tone === 'warn' ? '!' : '×';
  return `<div class="check"><div class="ic ${tone}">${ic}</div><div style="flex:1;min-width:0"><div class="t">${title}</div><div class="d">${desc}</div>${act ? `<div class="act">${act}</div>` : ''}</div></div>`;
}

function renderWizard() {
  const e = wiz.env;
  const c = wiz.cfg;
  const bar = `<div class="steps">${Array.from({ length: STEPS }, (_, i) => `<i class="${i <= wiz.step ? 'on' : ''}"></i>`).join('')}</div>`;
  let body = '';
  let next = `<button class="btn primary" data-act="wiz-next">下一步 ${icon.arrowRight()}</button>`;

  if (!e || !c) {
    body = `<h2>欢迎使用 Codex Bridge</h2><p class="lead">正在检测本机环境…</p><div class="result"><span class="spinner"></span></div>`;
    next = '';
  } else if (wiz.step === 0) {
    const auth = status?.auth ?? e.auth;
    const authOk = auth.kind !== 'missing';
    body = `<div><h2>检查环境</h2><p class="lead">需要 Codex CLI 已登录 ChatGPT 账号，并能修改 Claude Code 的配置。</p></div>
      <div class="checks">
        ${checkRow(e.codexCli ? 'ok' : 'err', 'Codex CLI', e.codexCli ? esc(e.codexCli) : '未安装。安装命令：<code>npm i -g @openai/codex</code>')}
        ${checkRow(authOk ? 'ok' : 'err', 'ChatGPT 登录', authOk ? `<code>${esc(e.authPath)}</code>` : esc((auth as any).reason), authOk ? '' : `<button class="btn" data-act="login">${icon.terminal()} 打开终端运行 codex login</button><button class="btn ghost" data-act="wiz-recheck">${icon.refresh()} 重新检测</button>`)}
        ${checkRow(e.settingsOk ? 'ok' : 'err', 'Claude Code 配置', e.settingsOk ? `<code>${esc(e.claudeSettings)}</code>` : esc(e.settingsError ?? ''))}
      </div>`;
    if (!authOk || !e.settingsOk) next = `<button class="btn primary" disabled>下一步 ${icon.arrowRight()}</button>`;
  } else if (wiz.step === 1) {
    const t = wiz.test;
    const res = t === 'loading' ? `<div class="result"><span class="spinner"></span> 正在连接 chatgpt.com…</div>`
      : t ? `<div class="result ${t.ok ? 'ok' : 'err'}">${t.ok ? icon.check(14) : icon.alert(14)} ${esc(t.text)}</div>` : `<div class="result"></div>`;
    body = `<div><h2>出口网络</h2><p class="lead">代理需要访问 OpenAI。国内网络一般要填本机代理地址，例如 <code style="font-family:var(--mono)">http://127.0.0.1:7890</code>。</p></div>
      <div class="field"><label for="w-proxy">出口代理</label>
        <div class="row"><input id="w-proxy" class="input mono" placeholder="留空 = 直连" value="${esc(c.outbound_proxy ?? '')}">
        <button class="btn" data-act="wiz-test">测试连通</button></div>
        <span class="hint">${e.systemProxy ? `已检测到系统代理 ${esc(e.systemProxy)}` : '未检测到系统代理'}</span></div>${res}`;
  } else if (wiz.step === 2) {
    body = `<div><h2>运行方式</h2><p class="lead">代理只监听本机，端口被占用时可以换一个。</p></div>
      <div class="field" style="max-width:160px"><label for="w-port">端口</label><input id="w-port" class="input mono" inputmode="numeric" value="${c.port}"></div>
      <div class="section"><div class="content" style="padding-top:8px">
        <div class="toggle-row"><span>开机自启（启动后只在托盘）</span><label class="switch"><input type="checkbox" id="w-auto" ${c.autostart ? 'checked' : ''} aria-label="开机自启"><span></span></label></div>
      </div></div>`;
  } else {
    const p = status ? proxyInfo(status.proxy, c.port) : { tone: 'warn' as Tone, value: '启动中…', meta: '' };
    const models = status?.models ?? [];
    body = `<div><h2>启动代理</h2><p class="lead">代理就绪后会列出可用的 Codex 模型。</p></div>
      ${card(p.tone, icon.server(), '代理', p.value, p.meta, '')}
      <div class="section"><header>可用 Codex 模型 <span class="count">${models.length}</span></header>
        <div class="content">${models.length ? `<div class="chips">${models.map((m) => `<span class="chip">${esc(m)}</span>`).join('')}</div>` : `<div class="empty">等待代理就绪…</div>`}</div></div>
      <div class="section"><div class="content" style="padding-top:8px">
        <div class="toggle-row"><span>接管 Claude Code（写入 settings.json）</span><label class="switch"><input type="checkbox" id="w-take" ${c.takeover ? 'checked' : ''} aria-label="接管 Claude Code"><span></span></label></div>
        <div class="empty">退出客户端时会自动恢复原配置。</div></div></div>`;
    const ready = status?.proxy.kind === 'running' && models.length > 0;
    next = `<button class="btn primary" data-act="wiz-finish" ${ready ? '' : 'disabled'}>${icon.check()} 完成</button>`;
  }

  const back = wiz.step > 0 ? `<button class="btn ghost" data-act="wiz-back">${icon.arrowLeft()} 上一步</button>` : '';
  app.innerHTML = `
    <div class="top"><div class="brand">${icon.logo()}</div><div class="grow"><h1>Codex Bridge</h1><div class="sub">首次设置 · 第 ${wiz.step + 1} / ${STEPS} 步</div></div></div>
    <div class="wizard">${bar}${body}<div class="nav">${back}<span class="grow"></span>${next}</div></div>`;
}

function syncWizardInputs() {
  if (!wiz.cfg) return;
  const val = (id: string) => (document.getElementById(id) as HTMLInputElement | null);
  const proxy = val('w-proxy');
  if (proxy) wiz.cfg.outbound_proxy = proxy.value.trim() || null;
  const port = val('w-port');
  if (port) {
    const n = Number(port.value);
    if (Number.isInteger(n) && n > 0 && n < 65536) wiz.cfg.port = n;
  }
  const auto = val('w-auto');
  if (auto) wiz.cfg.autostart = auto.checked;
  const take = val('w-take');
  if (take) wiz.cfg.takeover = take.checked;
}

// ───────── 渲染与事件 ─────────
function render() {
  if (!status) return;
  if (!status.onboarded) renderWizard();
  else renderMain(status);
}

// 引导第 2、3 步有输入框，状态推送时不重绘，避免冲掉输入
function shouldRerender() {
  if (status && !status.onboarded) return wiz.step === 0 || wiz.step === 3;
  const ae = document.activeElement;
  return !(ae instanceof HTMLInputElement && ae.type !== 'checkbox');
}

let toastTimer = 0;
function toast(text: string) {
  let el = document.querySelector('.toast') as HTMLElement | null;
  if (!el) {
    el = document.createElement('div');
    el.className = 'toast';
    document.body.appendChild(el);
  }
  el.textContent = text;
  el.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => el!.classList.remove('show'), 1800);
}

async function act(name: string, target: HTMLElement) {
  try {
    switch (name) {
      case 'restart': await invoke('restart_proxy'); toast('正在重启代理'); break;
      case 'kill-port': await invoke('kill_port_owner'); toast('正在接管端口'); break;
      case 'login': await invoke('open_codex_login'); toast('已打开终端，登录完成后会自动检测'); break;
      case 'takeover': await invoke('set_takeover', { on: (target as HTMLInputElement).checked }); break;
      case 'refresh-models': { const m = await invoke<string[]>('refresh_models'); toast(`已刷新，共 ${m.length} 个模型`); break; }
      case 'copy-dev-cmd': await navigator.clipboard.writeText(target.dataset.cmd ?? ''); toast('已复制命令'); break;
      case 'copy-logs': { const n = await invoke<number>('copy_logs'); toast(`已复制 ${n} 行日志`); break; }
      case 'settings-reset': settingsDraft = null; render(); break;
      case 'settings-save': {
        const cfg = readForm(status!.config);
        await invoke('save_config', { cfg });
        settingsDraft = null;
        toast('已保存');
        break;
      }
      case 'wiz-next': syncWizardInputs(); wiz.step++; if (wiz.step === 3) startPreview(); render(); break;
      case 'wiz-back': syncWizardInputs(); wiz.step--; render(); break;
      case 'wiz-recheck': await loadEnv(); break;
      case 'wiz-test': {
        syncWizardInputs();
        wiz.test = 'loading'; render();
        try {
          const ms = await invoke<number>('test_outbound', { proxy: wiz.cfg!.outbound_proxy });
          wiz.test = { ok: true, text: `连通，耗时 ${ms} ms` };
        } catch (e) {
          wiz.test = { ok: false, text: String(e) };
        }
        render();
        break;
      }
      case 'wiz-finish': syncWizardInputs(); await invoke('finish_onboarding', { cfg: wiz.cfg }); toast('设置完成'); break;
    }
  } catch (e) {
    toast(String(e));
  }
}

async function startPreview() {
  await invoke('preview_start', { cfg: wiz.cfg });
}

app.addEventListener('click', (e) => {
  const el = (e.target as HTMLElement).closest('[data-act]') as HTMLElement | null;
  if (!el || el.tagName === 'INPUT') return;
  e.preventDefault();
  if ((el as HTMLButtonElement).disabled) return;
  act(el.dataset.act!, el);
});
app.addEventListener('change', (e) => {
  const el = e.target as HTMLElement;
  if (el.tagName === 'INPUT' && el.dataset.act) act(el.dataset.act, el);
  if (el.id === 'w-take' || el.id === 'w-auto') syncWizardInputs();
});

onStatus('status-changed', (s) => {
  const wasOnboarded = status?.onboarded;
  status = s;
  if (wasOnboarded === false && s.onboarded) settingsDraft = null;
  if (shouldRerender() || wasOnboarded !== s.onboarded) render();
});

(async () => {
  status = await invoke<Status>('get_status');
  if (!status.onboarded) {
    render();
    await loadEnv();
  } else render();
})();
