// 界面：方案 A「一键开关」——主页一个大开关 + 底部四个标签（主页 / 模型 / 动态 / 设置）；首次引导是一页自动检测 + 一键接管。
import { invoke, onEvent, onStatus, type Activity, type AppConfig, type AuthState, type EnvReport, type CustomApiDto, type CustomApiStepEvent, type Overview, type Status, type StepId, type StepResult, type UpdateInfo, type VerifyReport } from './api';
import { icon } from './icons';
import { LANGS, setLang, t, trError } from './i18n';
import { toast } from './toast';
import { mockStep } from './mock';
import { anySelectOpen, installSelects, renderSelect, selectValue } from './select';

type Tone = 'ok' | 'warn' | 'err';
type Tab = 'home' | 'models' | 'feed' | 'settings';
const APP_NAME = 'CC Bridge';
const DEFAULT_MODELS = 6;
const HOME_MODELS = 8;
const app = document.getElementById('app')!;
const esc = (s: unknown) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);

// logo：白色 >（Codex）接陶土橙 >（Claude Code）
const LOGO = `<svg class="logo" viewBox="0 0 100 100" aria-hidden="true"><rect x="4" y="4" width="92" height="92" rx="22" fill="#141414"/>
<path d="M22 30 L42 50 L22 70" fill="none" stroke="#fff" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/>
<path d="M48 30 L68 50 L48 70" fill="none" stroke="#D97757" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/>
<rect x="72" y="62" width="10" height="8" rx="3" fill="#D97757"/></svg>`;

let status: Status | null = null;
let appVersion = '';
let update: UpdateInfo | null = null;
let updatePct: number | null = null;
let sheetOpen = false;
let checking = false;
let activity: Overview | null = null;

// 「自己的 Claude API」弹窗状态；密钥只存在这里（内存）和输入框里，不显示、不记录
interface CapiForm { id: string | null; mask: string; url: string; key: string; model: string; running: boolean; steps: Partial<Record<StepId, StepResult>>; error: string | null; failed: boolean }
const CAPI_STEPS: StepId[] = ['auth', 'format', 'tokenizer', 'signature', 'modelName'];
let capi: CapiForm | null = null;
// 删除确认弹窗：待删除条目的 id 与模型名
let capiDel: { id: string; model: string } | null = null;

let tab: Tab = 'home';
let feedMode: 'req' | 'raw' = 'req';
let query = '';
const folds = new Map<string, boolean>(); // 模型分组：用户手动展开 / 收起过的
let seenErrAt = Date.now(); // 「动态」标签的红点：这之后有失败请求才亮
let netTest: null | 'run' | { ok: boolean; text: string } = null;
const scrollTop: Partial<Record<Tab, number>> = {};

// ───────── 工具 ─────────
const pad = (n: number) => String(n).padStart(2, '0');
const fmtDay = (sec: number) => { const d = new Date(sec * 1000); return `${pad(d.getMonth() + 1)}-${pad(d.getDate())}`; };
const fmtTime = (sec: number) => { const d = new Date(sec * 1000); return `${fmtDay(sec)} ${pad(d.getHours())}:${pad(d.getMinutes())}`; };
const fmtClock = (ms: number) => { const d = new Date(ms); return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`; };
const fmtSec = (ms: number) => `${(ms / 1000).toFixed(1)}s`;
const authReason = (a: AuthState) => (a.kind === 'missing' ? t(`authCode_${a.code}` as any) || a.code : '');
const sw = (on: boolean, act: string, label: string, extra = '') =>
  `<button class="sw ${on ? 'on' : ''}" role="switch" aria-checked="${on}" aria-label="${esc(label)}" data-act="${act}" ${extra}></button>`;

// ───────── 状态推导 ─────────
interface Issue { tone: 'warn' | 'err'; title: string; body: string; acts: [string, string][] }

function issues(s: Status): Issue[] {
  const out: Issue[] = [];
  const p = s.proxy;
  if (p.kind === 'portBusy') {
    out.push({
      tone: 'err', title: t('portBusy', { port: s.port }),
      body: p.isClaudeCodex ? t('portBusyHintOurs') : `${t('portBusyOther', { pid: p.pid ?? t('unknown') })} ${t('portBusyHintOther')}`,
      acts: p.isClaudeCodex ? [['kill-port', t('killAndTake')], ['go-settings', t('changePort')]] : [['go-settings', t('changePort')]],
    });
  }
  if (p.kind === 'failed') out.push({ tone: 'err', title: t('failedTitle'), body: t('failedBody'), acts: [['restart', t('retry')], ['go-raw', t('viewLogs')]] });
  if (s.authHint || s.auth.kind === 'missing') {
    out.push({ tone: 'warn', title: t('loginTitle'), body: s.authHint ? t('loginBody401') : authReason(s.auth), acts: [['login', t('reLogin')]] });
  }
  if (s.settingsError) out.push({ tone: 'err', title: t('settingsErrTitle'), body: trError('settingsErr_', s.settingsError), acts: [] });
  if (s.networkHint) out.push({ tone: 'warn', title: t('netTitle'), body: t('netBody'), acts: [['go-settings', t('checkProxy')]] });
  return out;
}

function hero(s: Status) {
  const iss = issues(s);
  const n = s.selectedModels.length;
  if (iss.length) {
    const down = s.proxy.kind === 'portBusy' || s.proxy.kind === 'failed';
    return { cls: 'warn', title: down ? t('heroDown') : t('heroIssue', { n: iss.length }), sub: t('heroIssueSub') };
  }
  if (s.proxy.kind === 'starting' || s.proxy.kind === 'restarting') return { cls: 'warn busy', title: t('heroStarting'), sub: t('heroStartingSub') };
  if (s.proxy.kind === 'stopped') return { cls: 'off', title: t('heroDown'), sub: t('heroStoppedSub') };
  if (s.takeover && !s.takenOver) return { cls: 'on busy', title: t('waiting'), sub: t('waitingMeta') };
  return s.takeover
    ? { cls: 'on', title: t('heroOn'), sub: t('heroOnSub', { n }) }
    : { cls: 'off', title: t('heroOff'), sub: t('heroOffSub') };
}

function chips(s: Status) {
  const a = s.auth;
  const codex: [Tone | '', string] = s.authHint || a.kind === 'missing' ? ['err', t('needLogin')]
    : a.kind === 'expiredRefreshable' ? ['warn', t('authRefresh')] : ['ok', t('chipAuthOk', { time: fmtDay(a.expiresAt) })];
  const p = s.proxy.kind;
  const proxy: [Tone | '', string] = p === 'running' ? ['ok', `:${s.port}`] : p === 'starting' || p === 'restarting' ? ['warn', t('proxyStarting')] : ['err', t('chipProxyDown')];
  const cfg: [Tone | '', string] = s.settingsError ? ['err', t('chipErr')] : s.takenOver ? ['ok', t('chipWritten')] : s.takeover ? ['warn', t('waiting')] : ['', t('chipOriginal')];
  const chip = (k: string, [tone, v]: [Tone | '', string]) => `<span class="hchip"><span class="dot ${tone}"></span>${k} · ${esc(v)}</span>`;
  return chip(t('chipCodex'), codex) + chip(t('chipProxy'), proxy) + chip(t('chipClaude'), cfg);
}

// 模型分组：最新大版本按小版本分组（GPT-6.2 / GPT-6.1 / GPT-6），更老的按大版本归到一组（GPT-5.x）
function families(models: string[]) {
  const ver = (id: string) => /^gpt-(\d+)(?:\.(\d+))?/i.exec(id);
  const maxMajor = Math.max(0, ...models.map((m) => Number(ver(m)?.[1] ?? 0)));
  const groups = new Map<string, string[]>();
  for (const m of models) {
    const v = ver(m);
    const f = !v ? t('otherModels') : Number(v[1]) === maxMajor ? `GPT-${v[1]}${v[2] ? '.' + v[2] : ''}` : `GPT-${v[1]}.x`;
    if (!groups.has(f)) groups.set(f, []);
    groups.get(f)!.push(m);
  }
  return [...groups.entries()];
}

const newErrors = () => (activity?.recent ?? []).some((r) => !r.ok && r.at > seenErrAt);

// ───────── 主界面 ─────────
function renderMain(s: Status) {
  const pill = update
    ? `<button class="pill upd" data-act="open-sheet">${icon.download(13)} ${esc(t('updPill', { version: update.version }))}</button>` : '';
  const body = tab === 'home' ? homeTab(s) : tab === 'models' ? modelsTab(s) : tab === 'feed' ? feedTab(s) : settingsTab(s);
  const tabs: [Tab, string, string][] = [['home', icon.home(20), t('tabHome')], ['models', icon.layers(20), t('tabModels')], ['feed', icon.activity(20), t('tabFeed')], ['settings', icon.settings(20), t('tabSettings')]];
  app.innerHTML = `
    <header class="hd">${LOGO}<b>${APP_NAME}</b>${s.devMode ? '<span class="tag">DEV</span>' : ''}<span class="sp"></span>${pill}</header>
    <main class="body" id="body">${body}</main>
    <nav class="tabs" role="tablist">${tabs.map(([k, ic, l]) => `<button role="tab" aria-selected="${tab === k}" class="${tab === k ? 'on' : ''}" data-act="tab" data-tab="${k}">${ic}<span>${l}</span>${
      k === 'feed' && newErrors() ? '<i class="badge"></i>' : k === 'settings' && update ? '<i class="badge acc"></i>' : ''}</button>`).join('')}</nav>
    ${sheetOpen && update ? updateSheet(update) : ''}${capi ? capiSheet(capi) : ''}${capiDel ? capiDelSheet(capiDel) : ''}`;
  const logs = document.getElementById('logs');
  if (logs) logs.scrollTop = logs.scrollHeight;
}

function homeTab(s: Status) {
  const h = hero(s);
  const sel = new Set(s.selectedModels);
  const today = activity?.today;
  const dev = s.devMode ? (() => {
    const cmd = `claude --settings "${s.settingsPath}"`;
    return `<div class="issue info">${icon.terminal(18)}<div class="tx"><b>${t('devTitle')}</b><div class="d">${t('devBody')}</div><code>${esc(cmd)}</code>
      <div class="acts"><button class="btn" data-act="copy-dev-cmd" data-cmd="${esc(cmd)}">${icon.copy(14)} ${t('copy')}</button></div></div></div>`;
  })() : '';
  return `
    <section class="card hero">
      <button class="power ${h.cls}" data-act="power" aria-label="${t('takeoverTitle')}">${icon.power(46)}</button>
      <h2>${esc(h.title)}</h2><p>${h.sub}</p>
      <div class="health">${chips(s)}</div>
    </section>
    ${issues(s).map((x) => `<div class="issue ${x.tone}">${icon.alert(18)}<div class="tx"><b>${esc(x.title)}</b><div class="d">${esc(x.body)}</div>
      ${x.acts.length ? `<div class="acts">${x.acts.map(([a, l], i) => `<button class="btn ${i === 0 ? 'dark' : ''}" data-act="${a}">${esc(l)}</button>`).join('')}</div>` : ''}</div></div>`).join('')}
    ${dev}
    <section class="card">
      <div class="sec-h"><b>${t('commonModels')} <small>${t('commonModelsHint')}</small></b>
        <button class="link" data-act="tab" data-tab="models">${t('allModels', { n: s.models.length })} ${icon.right(14)}</button></div>
      ${s.models.length ? `<div class="mchips">${s.models.slice(0, HOME_MODELS).map((m, i) => `<button class="mchip ${sel.has(m) ? 'on' : ''}" data-act="model" data-model="${esc(m)}" aria-pressed="${sel.has(m)}">${sel.has(m) ? icon.check(12) : ''}${esc(m)}${i === 0 ? `<span class="new">${t('latest')}</span>` : ''}</button>`).join('')}</div>`
        : `<div class="empty">${t('modelsEmpty')}</div>`}
    </section>
    <section class="card">
      <div class="sec-h"><b>${t('today')}</b><button class="link" data-act="tab" data-tab="feed">${t('viewFeed')} ${icon.right(14)}</button></div>
      <div class="stats">
        <div class="stat"><div class="k">${t('statReq')}</div><div class="v">${today?.count ?? 0}</div></div>
        <div class="stat"><div class="k">${t('statAvg')}</div><div class="v">${today?.count ? fmtSec(today.avgMs) : '—'}</div></div>
        <div class="stat"><div class="k">${t('statErr')}</div><div class="v ${today?.errors ? 'bad' : ''}">${today?.errors ?? 0}</div></div>
      </div>
    </section>`;
}

function modelsTab(s: Status) {
  const sel = new Set(s.selectedModels);
  const q = query.trim().toLowerCase();
  const use = activity?.modelUse ?? {};
  const isDefault = s.config.selected_models === null;
  const groups = families(s.models).map(([f, ms]) => [f, ms.filter((m) => m.toLowerCase().includes(q))] as const).filter(([, ms]) => ms.length);
  return `
    <div class="mtools">
      <label class="search">${icon.search(16)}<input id="q" data-in="q" placeholder="${t('searchModels')}" value="${esc(query)}" autocomplete="off" spellcheck="false"></label>
      <div class="quick">
        <button class="${isDefault ? 'on' : ''}" data-act="models-default">${t('recommend', { n: DEFAULT_MODELS })}</button>
        <button data-act="models-all">${t('selectAll')}</button><button data-act="models-none">${t('selectNone')}</button>
        <button data-act="refresh-models" title="${t('refresh')}" aria-label="${t('refresh')}">${icon.refresh(13)}</button>
        <span class="count">${t('selectedCount', { n: sel.size, total: s.models.length })}</span></div>
    </div>
    ${!s.models.length ? `<div class="card empty">${t('modelsEmpty')}</div>` : !groups.length ? `<div class="card empty">${t('noMatch')}</div>` : groups.map(([f, ms]) => {
      const on = ms.filter((m) => sel.has(m)).length;
      const closed = !q && (folds.get(f) ?? on === 0);
      return `<section class="grp ${closed ? 'closed' : ''}">
        <div class="grp-h" data-act="fold" data-fam="${esc(f)}" role="button" aria-expanded="${!closed}"><span class="gl">${icon.chevron(14)}${esc(f)}</span>
          <span class="gr"><span class="cnt ${on ? 'has' : ''}">${on}/${ms.length}</span><button class="all" data-act="group-toggle" data-fam="${esc(f)}">${on === ms.length ? t('groupAllOff') : t('groupAllOn')}</button></span></div>
        ${closed ? '' : ms.map((m) => `<div class="row"><span class="name">${esc(m)}</span>${m === s.models[0] ? `<span class="new">${t('latest')}</span>` : ''}
          <span class="meta">${use[m] ? t('use7d', { n: use[m] }) : t('neverUsed')}</span>${sw(sel.has(m), 'model', m, `data-model="${esc(m)}"`)}</div>`).join('')}
      </section>`;
    }).join('')}
    ${capiSection(s)}
    <p class="foot-note">${t('modelsFoot')}</p>`;
}

function hostOf(u: string) { try { return new URL(u).host; } catch { return u; } }

function capiSection(s: Status) {
  const rows = (s.customApis ?? []).map((c: CustomApiDto) => {
    const ok = c.verifiedAt !== null;
    return `<div class="crow"><div class="ci"><div class="cm"><span class="name">${esc(c.pickerName ?? `API ${c.model}`)}</span><span class="cbadge ${ok ? 'ok' : 'bad'}">${ok ? icon.check(11) : icon.close(11)}${t(ok ? 'capiVerified' : 'capiFailed')}</span></div>
      <div class="cs" title="${esc(c.baseUrl)}">${esc(hostOf(c.baseUrl))} · ${esc(c.keyMask)}</div></div>
      <div class="ca"><button data-act="capi-verify" data-id="${esc(c.id)}">${t('capiVerify')}</button><button data-act="capi-edit" data-id="${esc(c.id)}">${t('capiEdit')}</button><button class="del" data-act="capi-del" data-id="${esc(c.id)}">${t('capiDel')}</button></div></div>`;
  }).join('');
  return `<section class="capi"><div class="capi-h"><b>${t('capiTitle')}</b><button class="btn sm" data-act="capi-add">+ ${t('capiAdd')}</button></div>
    <p class="capi-hint">${t('capiHint')}</p>${rows}</section>`;
}

function capiSheet(f: CapiForm) {
  const lock = f.running ? 'disabled' : '';
  const first = CAPI_STEPS.find((id) => !f.steps[id]);
  const showSteps = f.running || Object.keys(f.steps).length > 0;
  const stepRow = (id: StepId) => {
    const r = f.steps[id];
    const mark = r ? (r.kind === 'pass' ? `<i class="sm ok">${icon.check(13)}</i>` : r.kind === 'fail' ? `<i class="sm bad">${icon.close(13)}</i>` : '<i class="sm skip">–</i>')
      : f.running && id === first ? '<i class="sm"><span class="spinner"></span></i>' : '<i class="sm idle"></i>';
    const why = r && r.kind !== 'pass' ? `<div class="why ${r.kind}">${esc(r.reason)}</div>` : '';
    return `<li class="${r ? r.kind : ''}">${mark}<div><span>${t(`capiStep_${id}` as const)}</span>${why}</div></li>`;
  };
  return `<div class="mask" data-act="capi-close"></div>
    <div class="sheet capi-sheet" role="dialog" aria-modal="true" aria-label="${esc(t('capiTitle'))}"><div class="grab"></div>
      <h3>${t('capiTitle')}</h3>
      <p class="capi-hint">${t('capiHint')}</p>
      <label class="fld"><span>${t('capiUrl')}</span><input class="v" id="capi-url" data-in="capi-url" placeholder="https://api.anthropic.com" value="${esc(f.url)}" ${lock} autocomplete="off" spellcheck="false"></label>
      <label class="fld"><span>${t('capiKey')}</span><input class="v" id="capi-key" type="password" data-in="capi-key" placeholder="${esc(f.id && f.mask ? t('capiKeyKeep', { mask: f.mask }) : 'sk-ant-…')}" value="${esc(f.key)}" ${lock} autocomplete="off" spellcheck="false"></label>
      <label class="fld"><span>${t('capiModel')}</span><input class="v" id="capi-model" data-in="capi-model" placeholder="claude-opus-5-5" value="${esc(f.model)}" ${lock} autocomplete="off" spellcheck="false"></label>
      ${f.error ? `<div class="capi-err">${esc(f.error)}</div>` : ''}
      ${showSteps ? `<ul class="steps">${CAPI_STEPS.map(stepRow).join('')}</ul>` : ''}
      <div class="sheet-acts"><button class="btn" data-act="capi-close" ${f.running ? 'disabled' : ''}>${t('capiCancel')}</button>
        <button class="btn pri grow" data-act="capi-save" ${lock}>${f.running ? '<span class="spinner"></span> ' : ''}${t('capiSave')}</button></div></div>`;
}

function capiDelSheet(d: { id: string; model: string }) {
  return `<div class="mask" data-act="capi-del-cancel"></div>
    <div class="sheet" role="alertdialog" aria-modal="true" aria-label="${esc(t('capiDelConfirm'))}"><div class="grab"></div>
      <h3>${esc(t('capiDelConfirm'))}</h3><div class="sheet-sub">${esc(d.model)}</div>
      <div class="sheet-acts"><button class="btn" data-act="capi-del-cancel">${t('capiCancel')}</button>
        <button class="btn danger grow" data-act="capi-del-ok">${t('capiDel')}</button></div></div>`;
}

async function capiRun(verifyId: string | null) {
  if (!capi) return;
  const f = capi;
  f.running = true; f.steps = {}; f.error = null; f.failed = false; render();
  try {
    const rep = verifyId
      ? await invoke<VerifyReport>('reverify_custom_api', { id: verifyId })
      : await invoke<VerifyReport>('save_custom_api', { input: { id: f.id, baseUrl: f.url.trim(), apiKey: f.key.trim() || null, model: f.model.trim() } });
    if (capi !== f) return;
    for (const [id, r] of rep.steps) f.steps[id] = r;
    f.running = false;
    if (rep.ok) { capi = null; toast(t('capiOk')); } else f.failed = true;
  } catch (e) {
    f.running = false; f.error = String(e).replace(/^Error:\s*/, '');
  }
  status = await invoke<Status>('get_status').catch(() => status);
  render();
}

function reqLine(r: Activity) {
  const why = r.ok ? null : r.httpStatus === 429 ? t('err429') : r.httpStatus === 401 ? t('err401') : r.httpStatus && r.httpStatus >= 500 ? t('err5xx') : (r.error ?? t('errOther'));
  const tokens = r.inputTokens != null || r.outputTokens != null ? t('tokensInOut', { in: (r.inputTokens ?? 0).toLocaleString(), out: (r.outputTokens ?? 0).toLocaleString() }) : '';
  return `<div class="req ${r.ok ? '' : 'bad'}">
    <div class="rl"><div><span class="m">${esc(r.model)}</span> <span class="route ${r.route}">${r.route === 'codex' ? 'Codex' : r.route === 'api' ? t('routeApi') : t('routeClaude')}</span></div>
      <div class="sub" title="${esc(r.error ?? '')}">${esc(why ?? tokens)}</div></div>
    <div class="st"><div>${r.ok ? fmtSec(r.ms) : (r.httpStatus ?? '!')}</div><div class="sub">${fmtClock(r.at)}</div></div></div>`;
}

function feedTab(s: Status) {
  const seg = `<div class="seg"><button class="${feedMode === 'req' ? 'on' : ''}" data-act="feed-mode" data-mode="req">${t('feedReq')}</button><button class="${feedMode === 'raw' ? 'on' : ''}" data-act="feed-mode" data-mode="raw">${t('feedRaw')}</button></div>`;
  if (feedMode === 'raw') {
    return `${seg}<div class="logs" id="logs">${s.logsTail.map(logLine).join('\n') || t('noLogs')}</div>
      <div><button class="btn" data-act="copy-logs">${icon.copy(14)} ${t('copyAll')}</button></div>`;
  }
  const list = activity?.recent ?? [];
  return `${seg}${list.length ? `<div class="feed">${list.map(reqLine).join('')}</div>` : `<div class="card empty">${t('feedEmpty')}</div>`}`;
}

function logLine(l: string) {
  const cls = /"level":"error"|failed|error|panic/i.test(l) ? 'e' : /"level":"warn"|in use|timed out|restart/i.test(l) ? 'w' : '';
  return cls ? `<span class="${cls}">${esc(l)}</span>` : esc(l);
}

function settingsTab(s: Status) {
  const c = s.config;
  const net = netTest === 'run' ? '<span class="res"><span class="spinner"></span></span>'
    : netTest ? `<span class="res ${netTest.ok ? 'ok' : 'err'}" title="${esc(netTest.text)}">${netTest.ok ? icon.check(13) : icon.alert(13)} ${esc(netTest.text)}</span>`
    : `<button class="btn sm" data-act="test-net">${t('testBtn')}</button>`;
  const langOptions = [{ value: '', label: t('langAuto') }, ...LANGS.map((l) => ({ value: l.code, label: l.name }))];
  const srow = (ic: string, title: string, sub: string, ctrl: string) =>
    `<div class="srow"><div class="ic">${ic}</div><div class="lb"><b>${title}</b>${sub ? `<span>${sub}</span>` : ''}</div>${ctrl}</div>`;
  return `
    <div class="sgroup-t">${t('setNet')}</div><section class="sgroup">
      ${srow(icon.globe(15), t('outbound'), t('outboundSub'), `<input class="v" id="f-proxy" data-save="outbound_proxy" placeholder="${t('outboundPh')}" value="${esc(c.outbound_proxy ?? '')}" spellcheck="false">${net}`)}
      ${srow(icon.server(15), t('port'), t('portSub'), `<input class="v short" id="f-port" data-save="port" inputmode="numeric" value="${c.port}">`)}
    </section>
    <div class="sgroup-t">${t('setGeneral')}</div><section class="sgroup">
      ${srow(icon.boot(15), t('autostart'), t('autostartSub'), sw(c.autostart, 'set-autostart', t('autostart')))}
      ${srow(icon.lang(15), t('language'), '', `<div class="selw">${renderSelect('f-lang', langOptions, c.language ?? '', t('language'))}</div>`)}
    </section>
    <div class="sgroup-t">${t('setAdvanced')}</div><section class="sgroup">
      ${srow(icon.folder(15), t('codexDir'), t('dirDefault', { dir: '~/.codex' }), `<input class="v" id="f-codex" data-save="codex_home" placeholder="${t('dirDefaultShort')}" value="${esc(c.codex_home ?? '')}" spellcheck="false">`)}
      ${srow(icon.folder(15), t('claudeDir'), t('dirDefault', { dir: '~/.claude' }), `<input class="v" id="f-claude" data-save="claude_dir" placeholder="${t('dirDefaultShort')}" value="${esc(c.claude_dir ?? '')}" spellcheck="false">`)}
    </section>
    <div class="sgroup-t">${t('setAbout')}</div>
    ${update ? updateCard(update) : `<section class="sgroup">${srow(icon.download(15), t('version', { v: appVersion }), t('autoCheck'),
      checking ? `<span class="res"><span class="spinner"></span> ${t('checking')}</span>` : `<button class="btn sm" data-act="check-update">${t('checkUpdate')}</button>`)}</section>`}`;
}

function progress() {
  const pct = updatePct ?? 0;
  return `<div class="bar"><i style="width:${pct}%"></i></div><div class="prog">${pct >= 100 ? t('updInstalling') : t('updDownloading', { pct })}</div>`;
}

function updateCard(u: UpdateInfo) {
  return `<section class="upcard"><div class="uh"><div><b>${esc(t('updTitle', { version: u.version }))}</b><span>${esc(t('updCurrent', { current: u.current }))}</span></div>
    ${updatePct === null ? `<button class="btn pri" data-act="install-update">${t('updAndRestart')}</button>` : ''}</div>
    ${updatePct !== null ? progress() : ''}</section>`;
}

function updateSheet(u: UpdateInfo) {
  return `<div class="mask" data-act="close-sheet"></div>
    <div class="sheet" role="dialog" aria-modal="true" aria-label="${esc(t('updTitle', { version: u.version }))}"><div class="grab"></div>
      <h3>${esc(t('updTitle', { version: u.version }))}</h3><div class="sheet-sub">${esc(t('updCurrent', { current: u.current }))}</div>
      <p>${t('updSheetBody')}</p>
      ${updatePct === null ? `<div class="sheet-acts"><button class="btn" data-act="close-sheet">${t('later')}</button><button class="btn pri grow" data-act="install-update">${t('updAndRestart')}</button></div>` : progress()}
    </div>`;
}

// ───────── 首次引导：一页自动检测 + 一键接管 ─────────
type Check = 'wait' | 'run' | 'ok' | 'fail';
const ob = {
  env: null as EnvReport | null,
  cfg: null as AppConfig | null,
  shown: 0, // 逐行出现的检测项数（0–4）
  net: 'wait' as 'wait' | 'run' | { ok: true; ms: number } | { ok: false; text: string },
  phase: 'check' as 'check' | 'starting' | 'done',
  takeover: true,
  startErr: '',
  timer: 0,
};
let obOptOpen = false;

async function loadEnv() {
  ob.shown = 0;
  ob.env = await invoke<EnvReport>('detect_environment');
  if (!ob.cfg) {
    ob.cfg = { ...(status?.config ?? { port: 18765, outbound_proxy: null, codex_home: null, claude_dir: null, autostart: true, takeover: true, selected_models: null, language: null }) };
    if (!ob.cfg.outbound_proxy && ob.env.systemProxy) ob.cfg.outbound_proxy = ob.env.systemProxy;
  }
  void testObNet();
  for (let i = 1; i <= 4; i++) setTimeout(() => { ob.shown = i; if (obVisible()) render(); }, i * 220);
}

async function testObNet() {
  ob.net = 'run';
  render();
  try {
    ob.net = { ok: true, ms: await invoke<number>('test_outbound', { proxy: ob.cfg?.outbound_proxy ?? null }) };
  } catch (e) {
    ob.net = { ok: false, text: trError('conn_', String(e)) };
  }
  render();
}

function obChecks(): Record<'cli' | 'login' | 'net' | 'cfg', Check> {
  const e = ob.env;
  const auth = status?.auth ?? e?.auth;
  const at = (i: number, v: Check): Check => (!e ? 'run' : ob.shown < i ? 'wait' : v);
  return {
    cli: at(1, e?.codexCli ? 'ok' : 'fail'),
    login: at(2, auth && auth.kind !== 'missing' ? 'ok' : 'fail'),
    net: at(3, ob.net === 'wait' || ob.net === 'run' ? 'run' : ob.net.ok ? 'ok' : 'fail'),
    cfg: at(4, e?.settingsOk ? 'ok' : 'fail'),
  };
}

const obVisible = () => !!status && (!status.onboarded || ob.phase === 'done');

function renderOnboarding(s: Status) {
  if (ob.phase === 'done') {
    const ms = s.selectedModels;
    app.innerHTML = `<main class="ob done">
      <div class="done-ic">${icon.check(38)}</div>
      <h1>${t('obDoneTitle')}</h1><p class="sub">${ob.takeover ? t('obDoneSub') : t('obDoneNoTake')}</p>
      ${ob.takeover ? `<div class="term"><div><span class="p">$</span> claude</div><div><span class="p">&gt;</span> /model</div>
        <div class="gap">  Default (Opus)</div><div>  Sonnet · Haiku</div><div class="p">  ── Codex ──</div>
        ${ms.slice(0, 4).map((m, i) => `<div class="${i === 0 ? 'sel' : ''}"><span class="hl">${i === 0 ? '❯' : ' '}</span> ${esc(m)}</div>`).join('')}
        ${ms.length > 4 ? `<div class="p">  ${t('moreN', { n: ms.length - 4 })}</div>` : ''}</div>` : ''}
      <div class="cta"><button class="btn big dark" data-act="ob-enter">${t('obEnter')}</button></div></main>`;
    return;
  }
  const c = obChecks();
  const e = ob.env;
  const auth = s.auth ?? e?.auth;
  const row = (st: Check, title: string, okText: string, failText: string, fix = '') => {
    const ic = st === 'ok' ? icon.check(13) : st === 'fail' ? icon.close(13) : st === 'run' ? '<span class="spinner"></span>' : '';
    return `<div class="chk"><span class="st ${st}">${ic}</span><div class="tx"><b>${title}</b>
      <div class="d">${st === 'ok' ? okText : st === 'fail' ? failText : st === 'run' ? t('obChecking') : t('obWaiting')}</div>${st === 'fail' && fix ? `<div class="fix">${fix}</div>` : ''}</div></div>`;
  };
  const netMs = typeof ob.net === 'object' && ob.net.ok ? ob.net.ms : 0;
  const via = ob.cfg?.outbound_proxy ? t('viaProxy', { proxy: esc(ob.cfg.outbound_proxy) }) : t('viaDirect');
  const recheck = `<button class="btn" data-act="ob-recheck">${icon.refresh(14)} ${t('recheck')}</button>`;
  const rows = [
    row(c.cli, 'Codex CLI', esc(e?.codexCli ?? ''), `${t('cliMissing')} <code>npm i -g @openai/codex</code>`,
      `<button class="btn dark" data-act="copy-dev-cmd" data-cmd="npm i -g @openai/codex">${icon.copy(14)} ${t('copy')}</button>${recheck}`),
    row(c.login, t('chatgptLogin'), auth?.kind === 'ok' ? t('chkLoginOk', { time: fmtTime(auth.expiresAt) }) : t('authRefresh'), esc(auth ? authReason(auth) : ''),
      `<button class="btn dark" data-act="login">${icon.terminal(14)} ${t('openLogin')}</button>${recheck}`),
    row(c.net, t('chkNet'), `${via} · ${netMs} ms`, esc(typeof ob.net === 'object' && !ob.net.ok ? ob.net.text : ''),
      `<input class="v" id="ob-proxy" placeholder="http://127.0.0.1:7890" value="${esc(ob.cfg?.outbound_proxy ?? '')}" spellcheck="false"><button class="btn dark" data-act="ob-net-test">${t('testBtn')}</button>`),
    row(c.cfg, t('claudeConfig'), t('chkCfgOk', { path: esc(e?.claudeSettings ?? '') }), esc(trError('settingsErr_', e?.settingsError ?? ''))),
  ].join('');
  const ready = Object.values(c).every((v) => v === 'ok');
  const starting = ob.phase === 'starting';
  const cfg = ob.cfg;
  const summary = t('obOptSummary', { auto: cfg?.autostart ? t('on') : t('off'), port: cfg?.port ?? '' });
  app.innerHTML = `<main class="ob">
    ${LOGO.replace('class="logo"', 'class="logo-big"')}
    <h1>${t('welcome')}</h1><p class="sub">${t('obWelcomeSub')}</p>
    <section class="card checks">${rows}</section>
    <details class="opt" id="ob-opt" ${obOptOpen ? 'open' : ''}><summary>${icon.chevron(14)} ${t('obOptions')} · ${esc(summary)}</summary>
      <section class="card sgroup">
        <div class="srow"><div class="lb"><b>${t('autostart')}</b><span>${t('autostartSub')}</span></div>${sw(!!cfg?.autostart, 'ob-auto', t('autostart'))}</div>
        <div class="srow"><div class="lb"><b>${t('port')}</b><span>${t('portSub')}</span></div><input class="v short" id="ob-port" inputmode="numeric" value="${cfg?.port ?? ''}"></div>
      </section></details>
    ${ob.startErr ? `<div class="issue err">${icon.alert(18)}<div class="tx"><b>${t('obStartFail')}</b><div class="d">${esc(ob.startErr)}</div></div></div>` : ''}
    <div class="cta">
      <button class="btn big pri" data-act="ob-go" ${ready && !starting ? '' : 'disabled'}>${starting ? `<span class="spinner"></span> ${t('obStarting')}` : ready ? `${icon.zap(16)} ${t('obGo')}` : `<span class="spinner"></span> ${t('obChecking')}`}</button>
      <button class="skip" data-act="ob-go-noto" ${ready && !starting ? '' : 'disabled'}>${t('obGoNoTake')}</button>
    </div></main>`;
}

function syncObInputs() {
  if (!ob.cfg) return;
  const v = (id: string) => (document.getElementById(id) as HTMLInputElement | null)?.value.trim();
  const px = v('ob-proxy');
  if (px !== undefined) ob.cfg.outbound_proxy = px || null;
  const port = Number(v('ob-port'));
  if (Number.isInteger(port) && port > 0 && port < 65536) ob.cfg.port = port;
}

async function obStart(takeover: boolean) {
  syncObInputs();
  ob.takeover = takeover;
  ob.startErr = '';
  ob.phase = 'starting';
  ob.cfg = { ...ob.cfg!, takeover };
  render();
  await invoke('preview_start', { cfg: ob.cfg });
  clearTimeout(ob.timer);
  ob.timer = window.setTimeout(() => { if (ob.phase === 'starting') { ob.phase = 'check'; ob.startErr = t('obStartTimeout'); render(); } }, 30000);
  await obProgress();
}

/** 引导点了「一键接管」后：代理就绪（有模型）就完成引导，出错就回到检测页显示原因 */
async function obProgress() {
  const s = status;
  if (ob.phase !== 'starting' || !s) return;
  if (s.proxy.kind === 'running' && s.models.length > 0) {
    clearTimeout(ob.timer);
    ob.phase = 'done';
    await invoke('finish_onboarding', { cfg: ob.cfg });
  } else if (s.proxy.kind === 'portBusy' || s.proxy.kind === 'failed') {
    clearTimeout(ob.timer);
    ob.phase = 'check';
    ob.startErr = s.proxy.kind === 'portBusy' ? `${t('portBusy', { port: s.port })} ${t('portBusyHintOther')}` : t('failedBody');
    obOptOpen = true;
  }
  render();
}

// ───────── 渲染与事件 ─────────
function render() {
  if (!status) return;
  setLang(status.lang);
  document.title = APP_NAME;
  const ae = document.activeElement as HTMLInputElement | null;
  const focusId = ae?.id && ae.tagName === 'INPUT' ? ae.id : null;
  const caret = focusId ? ae!.selectionStart : null;
  const body = document.getElementById('body');
  if (body) scrollTop[tab] = body.scrollTop;
  if (obVisible()) {
    renderOnboarding(status);
  } else {
    renderMain(status);
    const nb = document.getElementById('body');
    if (nb) nb.scrollTop = scrollTop[tab] ?? 0;
  }
  if (focusId) {
    const el = document.getElementById(focusId) as HTMLInputElement | null;
    if (el) { el.focus(); if (caret !== null) try { el.setSelectionRange(caret, caret); } catch { /* 数字框不支持 */ } }
  }
}

// 输入框有焦点、下拉展开时，后端状态推送不重绘，避免冲掉输入
function shouldRerender() {
  const ae = document.activeElement;
  return !anySelectOpen() && !(ae instanceof HTMLInputElement && ae.type !== 'checkbox');
}

async function setModels(models: string[] | null) {
  await invoke('set_models', { models: models && status ? status.models.filter((m) => models.includes(m)) : models });
}

async function saveConfig(patch: Partial<AppConfig>) {
  if (!status) return;
  const cfg = { ...status.config, ...patch };
  if (JSON.stringify(cfg) === JSON.stringify(status.config)) return;
  await invoke('save_config', { cfg });
  toast(t('toastSaved'));
}

async function act(name: string, el: HTMLElement): Promise<void> {
  const s = status!;
  try {
    switch (name) {
      case 'tab': {
        const next = el.dataset.tab as Tab;
        if (next === 'feed') seenErrAt = Date.now();
        if (next !== tab) scrollTop[next] = 0;
        tab = next;
        render();
        break;
      }
      case 'power': {
        const iss = issues(s);
        if (iss.length && iss[0].acts.length) return act(iss[0].acts[0][0], el);
        if (s.proxy.kind === 'stopped') return act('restart', el);
        if (s.proxy.kind !== 'running') return;
        const on = !s.takeover;
        await invoke('set_takeover', { on });
        toast(on ? t('toastTakeOn') : t('toastTakeOff'));
        break;
      }
      case 'restart': await invoke('restart_proxy'); toast(t('toastRestart'), 'loading'); break;
      case 'kill-port': await invoke('kill_port_owner'); toast(t('toastKill'), 'loading'); break;
      case 'login':
        try { await invoke('open_codex_login'); toast(t('toastLogin'), 'info'); } catch (e) { toast(trError('err_', String(e)), 'error'); }
        break;
      case 'go-settings': tab = 'settings'; scrollTop.settings = 0; render(); document.getElementById('f-port')?.focus(); break;
      case 'go-raw': tab = 'feed'; feedMode = 'raw'; render(); break;
      case 'copy-dev-cmd': await navigator.clipboard.writeText(el.dataset.cmd ?? ''); toast(t('toastCopiedCmd')); break;
      case 'model': {
        const cur = new Set(s.selectedModels);
        const m = el.dataset.model!;
        if (cur.has(m)) cur.delete(m); else cur.add(m);
        await setModels([...cur]);
        break;
      }
      case 'models-all': await setModels(s.models); break;
      case 'models-none': await setModels([]); break;
      case 'models-default': await setModels(null); break;
      case 'refresh-models': { const m = await invoke<string[]>('refresh_models'); toast(t('toastRefreshed', { n: m.length })); break; }
      case 'fold': {
        const f = el.dataset.fam!;
        const ms = families(s.models).find(([k]) => k === f)?.[1] ?? [];
        const closed = folds.get(f) ?? !ms.some((m) => s.selectedModels.includes(m));
        folds.set(f, !closed);
        render();
        break;
      }
      case 'group-toggle': {
        const ms = families(s.models).find(([k]) => k === el.dataset.fam)?.[1] ?? [];
        const cur = new Set(s.selectedModels);
        const all = ms.every((m) => cur.has(m));
        ms.forEach((m) => (all ? cur.delete(m) : cur.add(m)));
        folds.set(el.dataset.fam!, false);
        await setModels([...cur]);
        break;
      }
      case 'feed-mode': feedMode = el.dataset.mode as 'req' | 'raw'; render(); break;
      case 'copy-logs': { const n = await invoke<number>('copy_logs'); toast(t('toastCopiedLogs', { n })); break; }
      case 'test-net': {
        const proxy = (document.getElementById('f-proxy') as HTMLInputElement).value.trim() || null;
        netTest = 'run'; render();
        try { netTest = { ok: true, text: `${await invoke<number>('test_outbound', { proxy })} ms` }; }
        catch (e) { netTest = { ok: false, text: trError('conn_', String(e)) }; }
        render();
        break;
      }
      case 'set-autostart': await saveConfig({ autostart: !s.config.autostart }); break;
      case 'check-update':
        checking = true; render();
        try {
          update = await invoke<UpdateInfo | null>('check_update');
          if (!update) toast(t('toastUpToDate'));
        } catch (e) { toast(t('toastCheckFail', { detail: String(e) }), 'error'); }
        checking = false; render();
        break;
      case 'capi-add': capi = { id: null, mask: '', url: '', key: '', model: '', running: false, steps: {}, error: null, failed: false }; render(); break;
      case 'capi-edit': {
        const c = s.customApis.find((x) => x.id === el.dataset.id);
        if (c) { capi = { id: c.id, mask: c.keyMask, url: c.baseUrl, key: '', model: c.model, running: false, steps: {}, error: null, failed: false }; render(); }
        break;
      }
      case 'capi-verify': {
        const c = s.customApis.find((x) => x.id === el.dataset.id);
        if (c) { capi = { id: c.id, mask: c.keyMask, url: c.baseUrl, key: '', model: c.model, running: false, steps: {}, error: null, failed: false }; await capiRun(c.id); }
        break;
      }
      case 'capi-save': await capiRun(null); break;
      case 'capi-close': if (capi && !capi.running) { capi = null; render(); } break;
      case 'capi-del': {
        const c = s.customApis.find((x) => x.id === el.dataset.id);
        if (c) { capiDel = { id: c.id, model: c.model }; render(); }
        break;
      }
      case 'capi-del-cancel': capiDel = null; render(); break;
      case 'capi-del-ok': {
        const d = capiDel;
        capiDel = null;
        if (!d) break;
        try { await invoke('delete_custom_api', { id: d.id }); } catch (e) { toast(String(e).replace(/^Error:\s*/, ''), 'error'); }
        status = await invoke<Status>('get_status').catch(() => status);
        render();
        break;
      }
      case 'open-sheet': sheetOpen = true; render(); break;
      case 'close-sheet': if (updatePct === null) { sheetOpen = false; render(); } break;
      case 'install-update':
        updatePct = 0; render();
        try { await invoke('install_update'); } catch (e) {
          if (String(e) === 'busy') break; // 已经在下载 / 安装，保持进度
          updatePct = null; render();
          if (String(e) === 'upToDate') { update = null; sheetOpen = false; render(); toast(t('toastUpToDate')); }
          else toast(t('toastUpdateFail', { detail: String(e) }), 'error');
        }
        break;
      // 引导
      case 'ob-recheck': ob.env = null; render(); await loadEnv(); break;
      case 'ob-net-test': syncObInputs(); await testObNet(); break;
      case 'ob-auto': syncObInputs(); ob.cfg!.autostart = !ob.cfg!.autostart; render(); break;
      case 'ob-go': await obStart(true); break;
      case 'ob-go-noto': await obStart(false); break;
      case 'ob-enter': ob.phase = 'check'; tab = 'home'; render(); toast(t('toastDone')); break;
    }
  } catch (e) {
    toast(String(e), 'error');
  }
}

app.addEventListener('click', (e) => {
  const el = (e.target as HTMLElement).closest('[data-act]') as HTMLElement | null;
  if (!el || el.tagName === 'INPUT') return;
  e.preventDefault();
  if ((el as HTMLButtonElement).disabled) return;
  void act(el.dataset.act!, el);
});
app.addEventListener('input', (e) => {
  const el = e.target as HTMLInputElement;
  if (el.dataset.in === 'q') { query = el.value; render(); }
  if (el.id === 'f-proxy') netTest = null;
  if (capi && el.dataset.in === 'capi-url') capi.url = el.value;
  if (capi && el.dataset.in === 'capi-key') capi.key = el.value;
  if (capi && el.dataset.in === 'capi-model') capi.model = el.value;
});
// 设置项：输入框失焦 / 回车（change 事件）时自动保存；端口不合法不保存
app.addEventListener('change', (e) => {
  const el = e.target as HTMLInputElement;
  if (el.id === 'f-lang') return void saveConfig({ language: selectValue('f-lang') || null });
  const key = el.dataset?.save as keyof AppConfig | undefined;
  if (!key) return;
  const v = el.value.trim();
  if (key === 'port') {
    const n = Number(v);
    if (!Number.isInteger(n) || n <= 0 || n >= 65536) { el.classList.add('bad'); toast(t('invalidPort'), 'error'); return; }
    el.classList.remove('bad');
    return void saveConfig({ port: n });
  }
  void saveConfig({ [key]: v || null } as Partial<AppConfig>);
});
app.addEventListener('keydown', (e) => {
  const el = e.target as HTMLInputElement;
  if (e.key === 'Enter' && el.dataset?.save) el.blur();
});
app.addEventListener('toggle', (e) => { if ((e.target as HTMLElement).id === 'ob-opt') obOptOpen = (e.target as HTMLDetailsElement).open; }, true);

onStatus('status-changed', (s) => {
  const was = status;
  status = s;
  if (ob.phase === 'starting') return void obProgress();
  if (shouldRerender() || was?.onboarded !== s.onboarded || was?.lang !== s.lang) render();
});

installSelects();
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && capiDel) { capiDel = null; render(); return; }
  if (e.key === 'Escape' && capi && !capi.running) { capi = null; render(); return; }
  if (e.key === 'Escape' && sheetOpen && updatePct === null && !anySelectOpen()) { sheetOpen = false; render(); }
});
onEvent<UpdateInfo | null>('update-available', (u) => { update = u; if (status?.onboarded && shouldRerender()) render(); });
onEvent<number>('update-progress', (p) => {
  updatePct = p;
  const bar = document.querySelector('.bar i') as HTMLElement | null;
  if (bar) {
    bar.style.width = `${p}%`;
    const txt = document.querySelector('.prog');
    if (txt) txt.textContent = p >= 100 ? t('updInstalling') : t('updDownloading', { pct: p });
  } else if (shouldRerender()) render();
});
onEvent<CustomApiStepEvent>('custom-api-step', (ev) => {
  if (!capi || !capi.running) return;
  if (!capi.id) capi.id = ev.id; // 新条目校验失败时后端也会存下来；记住 id，同一弹窗里重试不会产生重复条目
  else if (capi.id !== ev.id) return;
  capi.steps[ev.step] = ev.result;
  render();
});
onEvent<Overview>('activity', (o) => {
  activity = o;
  if (status?.onboarded && tab !== 'settings' && shouldRerender()) render();
});

(async () => {
  status = await invoke<Status>('get_status');
  appVersion = await invoke<string>('app_version');
  update = await invoke<UpdateInfo | null>('get_update');
  activity = await invoke<Overview>('get_activity').catch(() => null);
  if (update?.installing) updatePct = 0;
  setLang(status.lang);
  // 仅浏览器预览（?mock=...）用：直接打开某个标签 / 更新面板，方便截图检查
  const q = new URLSearchParams(location.search);
  if (q.has('mock') && q.get('tab')) tab = q.get('tab') as Tab;
  if (q.has('mock') && q.get('feed') === 'raw') feedMode = 'raw';
  if (q.has('mock') && q.has('sheet')) sheetOpen = true;
  if (q.has('mock') && q.has('err')) seenErrAt = 0;
  if (q.has('mock') && q.has('capidel')) { const c = status.customApis[0]; if (c) capiDel = { id: c.id, model: c.model }; }
  const cp = q.has('mock') ? q.get('capi') : null; // 预览：progress = 校验中途，fail = 失败态，edit = 编辑态
  if (cp) {
    const c = status.customApis[cp === 'edit' ? 0 : 1];
    capi = { id: cp === 'edit' ? c.id : null, mask: c.keyMask, url: c.baseUrl, key: '', model: c.model, running: cp === 'progress', steps: {}, error: null, failed: cp === 'fail' };
    if (cp === 'progress') capi.steps = Object.fromEntries(mockStep(3, false)); else if (cp === 'fail') capi.steps = Object.fromEntries(mockStep(3, true));
  }
  render();
  if (!status.onboarded) await loadEnv();
})();
