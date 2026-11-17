import { invoke, onStatus, type AppConfig, type AuthState, type EnvReport, type ProxyState, type Status } from './api';
import { icon } from './icons';
import { setLang, t, trError } from './i18n';

type Tone = 'ok' | 'warn' | 'err';
type Info = { tone: Tone; value: string; meta: string };
const APP_NAME = 'CC Bridge';
const DEFAULT_MODELS = 6;
const app = document.getElementById('app')!;
const esc = (s: unknown) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);

let status: Status | null = null;

// ───────── 状态文案 ─────────
function proxyInfo(p: ProxyState, port: number): Info {
  switch (p.kind) {
    case 'running': return { tone: 'ok', value: t('proxyRunning'), meta: `127.0.0.1:${port}` };
    case 'starting': return { tone: 'warn', value: t('proxyStarting'), meta: `127.0.0.1:${port}` };
    case 'restarting': return { tone: 'warn', value: t('proxyRestarting', { n: p.attempt }), meta: t('proxyRestartingMeta') };
    case 'failed': return { tone: 'err', value: t('proxyFailed'), meta: t('proxyFailedMeta') };
    case 'portBusy': return {
      tone: 'err', value: t('portBusy', { port }),
      meta: p.isClaudeCodex ? t('portBusyOurs', { pid: p.pid ?? t('unknown') }) : t('portBusyOther', { pid: p.pid ?? t('unknown') }),
    };
    case 'stopped': return { tone: 'warn', value: t('proxyStopped'), meta: '' };
  }
}

function fmtTime(sec: number) {
  const d = new Date(sec * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

const authReason = (a: AuthState) => (a.kind === 'missing' ? t(`authCode_${a.code}` as any) || a.code : '');

function authInfo(a: AuthState, hint: string | null): Info {
  if (hint) return { tone: 'err', value: t('auth401'), meta: t('auth401Meta') };
  switch (a.kind) {
    case 'ok': return { tone: 'ok', value: t('authOk'), meta: t('authOkMeta', { time: fmtTime(a.expiresAt) }) };
    case 'expiredRefreshable': return { tone: 'warn', value: t('authRefresh'), meta: t('authRefreshMeta') };
    case 'missing': return { tone: 'err', value: t('authMissing'), meta: authReason(a) };
  }
}

function takeoverInfo(s: Status): Info {
  if (s.settingsError) return { tone: 'err', value: t('settingsErrTitle'), meta: trError('settingsErr_', s.settingsError) };
  if (s.takenOver) return { tone: 'ok', value: t('taken'), meta: t('takenMeta') };
  if (s.takeover) return { tone: 'warn', value: t('waiting'), meta: t('waitingMeta') };
  return { tone: 'warn', value: t('notTaken'), meta: t('notTakenMeta') };
}

// ───────── 主界面 ─────────
// logo：白色 >（Codex）接陶土橙 >（Claude Code）
const LOGO = `<svg class="logo" viewBox="0 0 100 100" aria-hidden="true"><rect x="4" y="4" width="92" height="92" rx="22" fill="#141414"/>
<path d="M22 30 L42 50 L22 70" fill="none" stroke="#fff" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/>
<path d="M48 30 L68 50 L48 70" fill="none" stroke="#D97757" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/>
<rect x="72" y="62" width="10" height="8" rx="3" fill="#D97757"/></svg>`;

let panel: 'none' | 'logs' | 'settings' = 'none';
let modelsExpanded = false;
const MODEL_PREVIEW = 8;

const topBar = (right: string, tag = '') =>
  `<div class="top">${LOGO}<b>${APP_NAME}</b>${tag ? `<span class="tag">${tag}</span>` : ''}<span class="grow"></span>${right}</div>`;

function renderMain(s: Status) {
  const p = proxyInfo(s.proxy, s.port);
  const a = authInfo(s.auth, s.authHint);
  const tk = takeoverInfo(s);
  const tones = [p.tone, a.tone, tk.tone];
  const overall: Tone = tones.includes('err') ? 'err' : tones.includes('warn') ? 'warn' : 'ok';
  const overallText = { ok: t('overallOk'), warn: t('overallWarn'), err: t('overallErr') }[overall];

  const alerts: string[] = [];
  if (s.proxy.kind === 'portBusy') {
    alerts.push(alert('err', t('portBusy', { port: s.port }), s.proxy.isClaudeCodex ? t('portBusyHintOurs') : t('portBusyHintOther'),
      s.proxy.isClaudeCodex ? `<button class="btn danger" data-act="kill-port">${t('killAndTake')}</button>` : ''));
  }
  if (s.proxy.kind === 'failed') {
    alerts.push(alert('err', t('failedTitle'), `${t('failedBody')}<br><code>${esc(s.proxy.message)}</code>`, `<button class="btn" data-act="restart">${t('retry')}</button>`));
  }
  if (s.authHint || s.auth.kind === 'missing') {
    alerts.push(alert('err', t('loginTitle'), s.authHint ? `<code>${esc(s.authHint)}</code>` : esc(authReason(s.auth)), `<button class="btn" data-act="login">codex login</button>`));
  }
  if (s.networkHint) alerts.push(alert('warn', t('netTitle'), `${t('netBody')}<br><code>${esc(s.networkHint)}</code>`, ''));
  if (s.devMode) {
    const cmd = `claude --settings "${s.settingsPath}"`;
    alerts.push(alert('info', t('devTitle'), `${t('devBody')}<br><code>${esc(cmd)}</code>`,
      `<button class="btn" data-act="copy-dev-cmd" data-cmd="${esc(cmd)}">${t('copy')}</button>`));
  }

  const cell = (k: string, i: Info, sub: string) =>
    `<div class="cell"><div class="k">${k}</div><div class="v"><span class="dot ${i.tone}"></span>${esc(i.value)}</div><div class="s" title="${esc(i.meta)}">${esc(sub)}</div></div>`;
  const authSub = s.auth.kind === 'ok' ? t('authExpiresAt', { time: fmtTime(s.auth.expiresAt) }) : a.tone === 'ok' ? '' : t('needLogin');
  const statusSec = `
    <section>
      <div class="h">${t('secStatus')}<span class="grow"></span><button class="act" data-act="restart">${icon.refresh(13)} ${t('restartProxy')}</button></div>
      <div class="grid">
        ${cell('Codex', a, authSub)}
        ${cell(t('secProxy'), { ...p, value: s.proxy.kind === 'running' ? `:${s.port}` : p.value }, s.proxy.kind === 'running' ? '127.0.0.1' : p.meta)}
        ${cell('Claude Code', tk, tk.tone === 'ok' ? t('takenSub') : '')}
      </div>
    </section>`;

  const take = `
    <section class="box tog">
      <div><b>${t('takeoverTitle')}</b><div class="sub">${esc(tk.tone === 'err' ? tk.meta : t('takeoverSub'))}</div></div>
      <label class="switch" title="${t('takeoverTitle')}"><input type="checkbox" data-act="takeover" ${s.takeover ? 'checked' : ''} aria-label="${t('takeoverTitle')}"><span></span></label>
    </section>`;

  const sel = new Set(s.selectedModels);
  const isDefault = s.config.selected_models === null;
  const shown = modelsExpanded ? s.models : s.models.slice(0, MODEL_PREVIEW);
  const rest = s.models.length - shown.length;
  const models = `
    <section>
      <div class="h">${t('secModels', { n: sel.size, total: s.models.length })}<span class="grow"></span>
        ${isDefault ? '' : `<button class="act" data-act="models-default">${t('resetDefault')}</button>`}
        <button class="act" data-act="models-all">${t('selectAll')}</button>
        <button class="act" data-act="models-none">${t('selectNone')}</button>
        <button class="act" data-act="refresh-models" title="${t('refresh')}" aria-label="${t('refresh')}">${icon.refresh(13)}</button></div>
      <div class="box models">
        ${s.models.length ? shown.map((m, i) => `
          <label class="m pick ${sel.has(m) ? 'on' : ''}">
            <input type="checkbox" data-act="model-toggle" data-model="${esc(m)}" ${sel.has(m) ? 'checked' : ''}>
            <span class="cb">${icon.check(12)}</span><span class="name">${esc(m)}</span>${i === 0 ? `<span class="new">${t('latest')}</span>` : ''}</label>`).join('') : `<div class="empty">${t('modelsEmpty')}</div>`}
        ${rest > 0 ? `<button class="more" data-act="models-more">${t('expandRest', { n: rest })} ${icon.chevron(14)}</button>` : ''}
        ${modelsExpanded && s.models.length > MODEL_PREVIEW ? `<button class="more" data-act="models-more">${t('collapse')}</button>` : ''}
      </div>
      <div class="note"><span>${t('modelsNote')}${isDefault ? t('modelsNoteDefault', { n: DEFAULT_MODELS }) : ''}</span></div>
    </section>`;

  const btns = `
    <div class="btns">
      <button class="btn ${panel === 'logs' ? 'on' : ''}" data-act="panel-logs">${icon.log(14)} ${t('logs')}</button>
      <button class="btn ${panel === 'settings' ? 'on' : ''}" data-act="panel-settings">${icon.settings(14)} ${t('settings')}</button>
    </div>`;

  const logs = panel === 'logs' ? `
    <section class="panel">
      <div class="logs" id="logs">${s.logsTail.map(logLine).join('\n') || t('noLogs')}</div>
      <div class="row" style="margin-top:8px;justify-content:flex-end">
        <button class="btn" data-act="copy-logs">${icon.copy(14)} ${t('copyAll')}</button></div>
    </section>` : '';

  app.innerHTML = `${topBar(`<span class="badge"><span class="dot ${overall}"></span>${overallText}</span>`, s.devMode ? 'DEV' : '')}
    <div class="main">${alerts.join('')}${statusSec}${take}${models}${btns}${logs}<div id="settings-slot"></div></div>
    <div class="foot">${t('foot')}</div>`;
  if (panel === 'settings') mountSettings(s.config);
  else document.getElementById('settings-slot')?.remove();
  const box = document.getElementById('logs');
  if (box) box.scrollTop = box.scrollHeight;
}

const alert = (tone: Tone | 'info', title: string, body: string, action: string) => `
  <div class="alert ${tone}">${icon.alert(16)}<div class="msg"><b>${esc(title)}</b>${body}</div>${action}</div>`;

function logLine(l: string) {
  const cls = /"level":"error"|failed|error|panic/i.test(l) ? 'e' : /"level":"warn"|in use|timed out|restart/i.test(l) ? 'w' : '';
  return cls ? `<span class="${cls}">${esc(l)}</span>` : esc(l);
}

// 设置面板：自己维护表单状态，不被状态推送覆盖
let settingsDraft: AppConfig | null = null;
function mountSettings(cfg: AppConfig) {
  const slot = document.getElementById('settings-slot')!;
  const d = settingsDraft ?? { ...cfg };
  const dirty = settingsDraft && JSON.stringify(settingsDraft) !== JSON.stringify(cfg);
  const opt = (v: string, label: string) => `<option value="${v}" ${(d.language ?? '') === v ? 'selected' : ''}>${label}</option>`;
  slot.outerHTML = `
    <section class="panel" id="settings">
      <div class="row">
        <div class="field" style="width:96px"><label for="f-port">${t('port')}</label><input id="f-port" class="input mono" inputmode="numeric" value="${d.port}"></div>
        <div class="field" style="flex:1"><label for="f-proxy">${t('outbound')}</label><input id="f-proxy" class="input mono" placeholder="${t('outboundPh')}" value="${esc(d.outbound_proxy ?? '')}"></div>
      </div>
      <div class="field"><label for="f-codex">${t('codexDir')}</label><input id="f-codex" class="input mono" placeholder="${t('dirDefault', { dir: '~/.codex' })}" value="${esc(d.codex_home ?? '')}"></div>
      <div class="field"><label for="f-claude">${t('claudeDir')}</label><input id="f-claude" class="input mono" placeholder="${t('dirDefault', { dir: '~/.claude' })}" value="${esc(d.claude_dir ?? '')}"></div>
      <div class="row">
        <div class="field" style="flex:1"><label for="f-lang">${t('language')}</label>
          <select id="f-lang" class="input">${opt('', t('langAuto'))}${opt('zh', '中文')}${opt('en', 'English')}</select></div>
        <div class="field" style="flex:1"><label>${t('autostart')}</label>
          <div style="height:32px;display:flex;align-items:center"><label class="switch"><input type="checkbox" id="f-auto" ${d.autostart ? 'checked' : ''} aria-label="${t('autostart')}"><span></span></label></div></div>
      </div>
      <div class="row" style="justify-content:flex-end;margin-top:8px;${dirty ? '' : 'display:none'}" id="settings-actions">
        <button class="btn ghost" data-act="settings-reset">${t('cancel')}</button>
        <button class="btn pri" data-act="settings-save">${t('saveApply')}</button>
      </div>
    </section>`;
  const el = document.getElementById('settings')!;
  const onInput = () => {
    settingsDraft = readForm(cfg);
    const changed = JSON.stringify(settingsDraft) !== JSON.stringify(cfg);
    (document.getElementById('settings-actions') as HTMLElement).style.display = changed ? '' : 'none';
  };
  el.addEventListener('input', onInput);
  el.addEventListener('change', onInput);
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
    language: v('f-lang') || null,
  };
}

// ───────── 首次运行引导 ─────────
const wiz = { step: 0, env: null as EnvReport | null, cfg: null as AppConfig | null, test: null as null | { ok: boolean; text: string } | 'loading' };
const STEPS = 4;

async function loadEnv() {
  wiz.env = await invoke<EnvReport>('detect_environment');
  if (!wiz.cfg) {
    wiz.cfg = { ...(status?.config ?? { port: 18765, outbound_proxy: null, codex_home: null, claude_dir: null, autostart: true, takeover: true, selected_models: null, language: null }) };
    if (!wiz.cfg.outbound_proxy && wiz.env.systemProxy) wiz.cfg.outbound_proxy = wiz.env.systemProxy;
  }
  render();
}

function checkRow(tone: Tone, title: string, desc: string, act = '') {
  return `<div class="check"><span class="dot ${tone}"></span><div style="flex:1;min-width:0"><div class="t">${title}</div><div class="d">${desc}</div>${act ? `<div class="act">${act}</div>` : ''}</div></div>`;
}

function renderWizard() {
  const e = wiz.env;
  const c = wiz.cfg;
  const bar = `<div class="steps">${Array.from({ length: STEPS }, (_, i) => `<i class="${i <= wiz.step ? 'on' : ''}"></i>`).join('')}</div>`;
  let body = '';
  let next = `<button class="btn pri" data-act="wiz-next">${t('next')} ${icon.arrowRight()}</button>`;

  if (!e || !c) {
    body = `<h2>${t('welcome')}</h2><p class="lead">${t('detecting')}</p><div class="result"><span class="spinner"></span></div>`;
    next = '';
  } else if (wiz.step === 0) {
    const auth = status?.auth ?? e.auth;
    const authOk = auth.kind !== 'missing';
    body = `<div><h2>${t('envTitle')}</h2><p class="lead">${t('envLead')}</p></div>
      <div class="checks">
        ${checkRow(e.codexCli ? 'ok' : 'err', 'Codex CLI', e.codexCli ? esc(e.codexCli) : `${t('cliMissing')} <code>npm i -g @openai/codex</code>`)}
        ${checkRow(authOk ? 'ok' : 'err', t('chatgptLogin'), authOk ? `<code>${esc(e.authPath)}</code>` : esc(authReason(auth)), authOk ? '' : `<button class="btn" data-act="login">${icon.terminal()} ${t('openLogin')}</button><button class="btn ghost" data-act="wiz-recheck">${icon.refresh()} ${t('recheck')}</button>`)}
        ${checkRow(e.settingsOk ? 'ok' : 'err', t('claudeConfig'), e.settingsOk ? `<code>${esc(e.claudeSettings)}</code>` : esc(trError('settingsErr_', e.settingsError ?? '')))}
      </div>`;
    if (!authOk || !e.settingsOk) next = `<button class="btn pri" disabled>${t('next')} ${icon.arrowRight()}</button>`;
  } else if (wiz.step === 1) {
    const r = wiz.test;
    const res = r === 'loading' ? `<div class="result"><span class="spinner"></span> ${t('testing')}</div>`
      : r ? `<div class="result ${r.ok ? 'ok' : 'err'}">${r.ok ? icon.check(14) : icon.alert(14)} ${esc(r.text)}</div>` : `<div class="result"></div>`;
    body = `<div><h2>${t('netStepTitle')}</h2><p class="lead">${t('netStepLead')} <code>http://127.0.0.1:7890</code></p></div>
      <div class="field"><label for="w-proxy">${t('outbound')}</label>
        <div class="row"><input id="w-proxy" class="input mono" placeholder="${t('outboundPh')}" value="${esc(c.outbound_proxy ?? '')}">
        <button class="btn" data-act="wiz-test">${t('testConn')}</button></div>
        <span class="hint">${e.systemProxy ? t('sysProxyFound', { proxy: esc(e.systemProxy) }) : t('sysProxyNone')}</span></div>${res}`;
  } else if (wiz.step === 2) {
    body = `<div><h2>${t('runTitle')}</h2><p class="lead">${t('runLead')}</p></div>
      <div class="field" style="max-width:160px"><label for="w-port">${t('port')}</label><input id="w-port" class="input mono" inputmode="numeric" value="${c.port}"></div>
      <div class="box tog"><div><b>${t('autostart')}</b><div class="sub">${t('autostartSub')}</div></div><label class="switch"><input type="checkbox" id="w-auto" ${c.autostart ? 'checked' : ''} aria-label="${t('autostart')}"><span></span></label></div>`;
  } else {
    const p = status ? proxyInfo(status.proxy, c.port) : { tone: 'warn' as Tone, value: t('proxyStarting'), meta: '' };
    const models = status?.models ?? [];
    body = `<div><h2>${t('startTitle')}</h2><p class="lead">${t('startLead')}</p></div>
      <div class="grid" style="grid-template-columns:1fr 1fr"><div class="cell"><div class="k">${t('secProxy')}</div><div class="v"><span class="dot ${p.tone}"></span>${esc(p.value)}</div><div class="s">${esc(p.meta)}</div></div>
        <div class="cell"><div class="k">${t('availableModels')}</div><div class="v">${models.length || '—'}</div><div class="s">${esc(models[0] ?? t('waitProxy'))}</div></div></div>
      <div class="box tog"><div><b>${t('takeoverTitle')}</b><div class="sub">${t('takeoverWizardSub')}</div></div><label class="switch"><input type="checkbox" id="w-take" ${c.takeover ? 'checked' : ''} aria-label="${t('takeoverTitle')}"><span></span></label></div>`;
    const ready = status?.proxy.kind === 'running' && models.length > 0;
    next = `<button class="btn pri" data-act="wiz-finish" ${ready ? '' : 'disabled'}>${icon.check()} ${t('done')}</button>`;
  }

  const back = wiz.step > 0 ? `<button class="btn ghost" data-act="wiz-back">${icon.arrowLeft()} ${t('back')}</button>` : '';
  app.innerHTML = `
    ${topBar(`<span class="badge">${t('setupBadge', { n: wiz.step + 1, total: STEPS })}</span>`)}
    <div class="wizard">${bar}${body}<div class="nav">${back}<span class="grow"></span>${next}</div></div>`;
}

function syncWizardInputs() {
  if (!wiz.cfg) return;
  const val = (id: string) => document.getElementById(id) as HTMLInputElement | null;
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
  setLang(status.lang);
  document.title = APP_NAME;
  if (!status.onboarded) renderWizard();
  else renderMain(status);
}

// 引导第 2、3 步有输入框，状态推送时不重绘，避免冲掉输入
function shouldRerender() {
  if (status && !status.onboarded) return wiz.step === 0 || wiz.step === 3;
  const ae = document.activeElement;
  return !((ae instanceof HTMLInputElement && ae.type !== 'checkbox') || ae instanceof HTMLSelectElement);
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
      case 'restart': await invoke('restart_proxy'); toast(t('toastRestart')); break;
      case 'kill-port': await invoke('kill_port_owner'); toast(t('toastKill')); break;
      case 'login':
        try { await invoke('open_codex_login'); toast(t('toastLogin')); } catch (e) { toast(trError('err_', String(e))); }
        break;
      case 'takeover': await invoke('set_takeover', { on: (target as HTMLInputElement).checked }); break;
      case 'refresh-models': { const m = await invoke<string[]>('refresh_models'); toast(t('toastRefreshed', { n: m.length })); break; }
      case 'copy-dev-cmd': await navigator.clipboard.writeText(target.dataset.cmd ?? ''); toast(t('toastCopiedCmd')); break;
      case 'model-toggle': {
        const cur = new Set(status!.selectedModels);
        const m = target.dataset.model!;
        (target as HTMLInputElement).checked ? cur.add(m) : cur.delete(m);
        await invoke('set_models', { models: status!.models.filter((x) => cur.has(x)) });
        break;
      }
      case 'models-all': await invoke('set_models', { models: status!.models }); break;
      case 'models-none': await invoke('set_models', { models: [] }); break;
      case 'models-default': await invoke('set_models', { models: null }); break;
      case 'models-more': modelsExpanded = !modelsExpanded; render(); break;
      case 'panel-logs': panel = panel === 'logs' ? 'none' : 'logs'; render(); break;
      case 'panel-settings': panel = panel === 'settings' ? 'none' : 'settings'; render(); break;
      case 'copy-logs': { const n = await invoke<number>('copy_logs'); toast(t('toastCopiedLogs', { n })); break; }
      case 'settings-reset': settingsDraft = null; render(); break;
      case 'settings-save': {
        const cfg = readForm(status!.config);
        await invoke('save_config', { cfg });
        settingsDraft = null;
        panel = 'none';
        toast(t('toastSaved'));
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
          wiz.test = { ok: true, text: t('connOk', { ms }) };
        } catch (e) {
          wiz.test = { ok: false, text: trError('conn_', String(e)) };
        }
        render();
        break;
      }
      case 'wiz-finish': syncWizardInputs(); await invoke('finish_onboarding', { cfg: wiz.cfg }); toast(t('toastDone')); break;
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
  const langChanged = status?.lang !== s.lang;
  status = s;
  if (wasOnboarded === false && s.onboarded) settingsDraft = null;
  if (shouldRerender() || wasOnboarded !== s.onboarded || langChanged) render();
});

(async () => {
  status = await invoke<Status>('get_status');
  setLang(status.lang);
  if (!status.onboarded) {
    render();
    await loadEnv();
  } else render();
})();
