// 界面：方案 A「一键开关」——主页一个大开关 + 底部四个标签（主页 / 模型 / 动态 / 设置）；首次引导是一页自动检测 + 一键接管。
import { invoke, onEvent, onStatus, type Activity, type AppConfig, type AuthState, type EnvReport, type ClaudeCodeCheck, type CustomApiDto, type CustomApiStepEvent, type Overview, type Status, type StepId, type StepResult, type UpdateInfo, type UsageModelRow, type UsageReport, type UsageTotals, type VerifyReport } from './api';
import { icon } from './icons';
import { LANGS, setLang, t, trError } from './i18n';
import { toast } from './toast';
import { mockStep } from './mock';
import { anySelectOpen, installSelects, renderSelect, selectValue } from './select';
import { chartInput, mountUsageChart } from './usage-chart';

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
/** 本机 Claude Code 版本；启动时查一次，主界面「重新检测」或引导页重新检测时刷新 */
let cc: ClaudeCodeCheck | null = null;
const CC_UPDATE_CMD = 'claude update';

// 「自己的 Claude API」弹窗状态；密钥只存在这里（内存）和输入框里，不显示、不记录
interface CapiForm { id: string | null; mask: string; url: string; key: string; model: string; running: boolean; steps: Partial<Record<StepId, StepResult>>; error: string | null; failed: boolean }
const CAPI_STEPS: StepId[] = ['auth', 'format'];
let capi: CapiForm | null = null;
// 删除确认弹窗：待删除条目的 id 与模型名
let capiDel: { id: string; model: string } | null = null;

let tab: Tab = 'home';
let feedMode: 'stats' | 'req' | 'raw' = 'stats';
// 用量统计：当前报表、筛选条件、请求序号（丢弃过期响应）、上次请求时间（节流）
let usage: UsageReport | null = null;
let usageErr: string | null = null;
const usageQ: { gran: 'today' | 'day' | 'month'; range: 7 | 30 | 90 | 'custom'; from: string; to: string; route: string; model: string; bucket: string | null } =
  { gran: 'day', range: 30, from: '', to: '', route: '', model: '', bucket: null };
let usageSeq = 0;
let usageAt = 0;
let usageTimer = 0;
let usageDateTimer = 0; // 自定义日期框 change 防抖
const USAGE_DATE_DEBOUNCE = 700;
const USAGE_MIN_GAP = 3000; // activity 事件触发的刷新至少间隔 3 秒
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
  // 只在找到了 claude 命令且版本过旧时提示：没装命令行的可能只用编辑器插件，不打扰
  if (cc && cc.version && !cc.ok) {
    out.push({ tone: 'warn', title: t('ccOldTitle'), body: `${t('ccOldBody', { version: cc.version, min: cc.min })} ${CC_UPDATE_CMD}`, acts: [['copy-cc-update', t('copy')], ['cc-recheck', t('recheck')]] });
  }
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
const ANIMATED = '.mask, .sheet, .issue, .chk .st.ok, .chk .st.fail, .chk .st.warn, .chk .fix, .done-ic';
/** 动画元素的身份：类名 + 文本。抽屉、遮罩只认类名（内容随进度变化也算同一个）；步骤图标看所在那一行的文字。 */
function animKey(n: Element): string {
  const c = [...n.classList].filter((x) => x !== 'still').join('.');
  if (n.matches('.mask, .sheet')) return c;
  const host = n.matches('.st') ? n.parentElement : n;
  return `${c}|${(host?.textContent ?? '').trim().slice(0, 160)}`;
}

function renderMain(s: Status) {
  const pill = update
    ? `<button class="pill upd" data-act="open-sheet">${icon.download(13)} ${esc(t('updPill', { version: update.version }))}</button>` : '';
  const body = tab === 'home' ? homeTab(s) : tab === 'models' ? modelsTab(s) : tab === 'feed' ? feedTab(s) : settingsTab(s);
  const tabs: [Tab, string, string][] = [['home', icon.home(20), t('tabHome')], ['models', icon.layers(20), t('tabModels')], ['feed', icon.activity(20), t('tabFeed')], ['settings', icon.settings(20), t('tabSettings')]];
  // 整页重绘会重建带入场动画的元素，重绘前已在屏上的不再重播动画，否则周期刷新时会一直闪；新出现的照常播放。
  const seen = new Set([...app.querySelectorAll(ANIMATED)].map(animKey));
  app.innerHTML = `
    <header class="hd">${LOGO}<b>${APP_NAME}</b>${s.devMode ? '<span class="tag">DEV</span>' : ''}<span class="sp"></span>${pill}</header>
    <main class="body" id="body">${body}</main>
    <nav class="tabs" role="tablist">${tabs.map(([k, ic, l]) => `<button role="tab" aria-selected="${tab === k}" class="${tab === k ? 'on' : ''}" data-act="tab" data-tab="${k}">${ic}<span>${l}</span>${
      k === 'feed' && newErrors() ? '<i class="badge"></i>' : k === 'settings' && update ? '<i class="badge acc"></i>' : ''}</button>`).join('')}</nav>
    ${sheetOpen && update ? updateSheet(update) : ''}${capi ? capiSheet(capi) : ''}${capiDel ? capiDelSheet(capiDel) : ''}`;
  app.querySelectorAll(ANIMATED).forEach((n) => { if (seen.has(animKey(n))) n.classList.add('still'); });
  // 常驻图表搬到新占位上（不在统计页时没有占位，什么都不做）；按报表自身粒度补桶，切粒度后新报表回来前不会错位
  if (usage) mountUsageChart(document.getElementById('usage-chart-slot'), chartInput(usage, usage.granularity, usageQ.bucket),
    (k) => { usageQ.bucket = k === usageQ.bucket ? null : k; render(); });
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
  const segBtn = (m: typeof feedMode, label: string) => `<button class="${feedMode === m ? 'on' : ''}" data-act="feed-mode" data-mode="${m}">${label}</button>`;
  const seg = `<div class="seg">${segBtn('stats', t('feedStats'))}${segBtn('req', t('feedReq'))}${segBtn('raw', t('feedRaw'))}</div>`;
  if (feedMode === 'stats') return `${seg}${statsView(s)}`;
  if (feedMode === 'raw') {
    return `${seg}<div class="logs" id="logs">${s.logsTail.map(logLine).join('\n') || t('noLogs')}</div>
      <div><button class="btn" data-act="copy-logs">${icon.copy(14)} ${t('copyAll')}</button></div>`;
  }
  const list = activity?.recent ?? [];
  return `${seg}${list.length ? `<div class="feed">${list.map(reqLine).join('')}</div>` : `<div class="card empty">${t('feedEmpty')}</div>`}`;
}

// ───────── 用量统计 ─────────
const ROUTES = ['claude', 'codex', 'api'] as const;
const routeName = (r: string) => (r === 'claude' ? t('routeClaude') : r === 'codex' ? t('routeGpt') : r === 'api' ? t('routeApi') : r);
const ymd = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const daysAgo = (n: number) => { const d = new Date(); d.setDate(d.getDate() - n); return ymd(d); };
/** 金额：$ + 两位小数；大于 0 但不到 1 美分显示 <$0.01 */
const fmtUsd = (v: number) => (v > 0 && v < 0.01 ? '<$0.01' : `$${v.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`);
/** Token 数：K / M / B 缩写，保留一位小数（整数去掉 .0） */
function fmtTok(n: number) {
  const units: [number, string][] = [[1, ''], [1e3, 'K'], [1e6, 'M'], [1e9, 'B']];
  const show = (v: number) => (Math.abs(v) >= 100 ? v.toFixed(0) : v.toFixed(1).replace(/\.0$/, ''));
  let i = units.length - 1;
  while (i > 0 && Math.abs(n) < units[i][0]) i--;
  // 四舍五入后到了 1000（如 999,950 → 1000K）就进到下一档
  while (i < units.length - 1 && Math.abs(Number(show(n / units[i][0]))) >= 1000) i++;
  return i === 0 ? String(n) : `${show(n / units[i][0])}${units[i][1]}`;
}
/** 自定义日期：完整的 YYYY-MM-DD 且年份 ≥ 2000（逐位输入年份时的 0002、0020 等中间值不算） */
const okDate = (v: string) => /^\d{4}-\d{2}-\d{2}$/.test(v) && Number(v.slice(0, 4)) >= 2000;
/** 后端错误 → 界面文案：日期格式错误本地化，其余原样 */
const usageErrText = (e: unknown) => {
  const raw = String(e).replace(/^Error:\s*/, '');
  const m = /^日期格式错误[：:]\s*(.*)$/.exec(raw);
  return m ? t('usageBadDate', { detail: m[1] }) : raw;
};
const fmtNum = (v: number) => (v >= 10 || Number.isInteger(v) ? Math.round(v).toLocaleString() : v.toFixed(1));
const tokIn = (x: UsageTotals) => x.input + x.cacheRead + x.cacheWrite;
const tokAll = (x: UsageTotals) => tokIn(x) + x.output;
/** 缓存命中率 = 缓存读 ÷（未命中输入 + 缓存读 + 缓存写）；分母为 0 返回 null */
const hitRate = (x: UsageTotals) => (tokIn(x) ? x.cacheRead / tokIn(x) : null);
const fmtPct = (v: number | null) => (v === null ? '—' : `${(v * 100).toFixed(1)}%`);
const statsVisible = () => !!status?.onboarded && !obVisible() && tab === 'feed' && feedMode === 'stats';

/** 当前筛选 → 后端参数；「今天」= from=to=今天、按日 */
function usageArgs() {
  const q = usageQ;
  const today = ymd(new Date());
  if (q.gran === 'today') return { from: today, to: today, granularity: 'day', route: q.route || null, model: q.model || null };
  const custom = q.range === 'custom';
  const from = custom ? q.from || daysAgo(29) : daysAgo((q.range as number) - 1);
  const to = custom ? q.to || today : today;
  return { from, to, granularity: q.gran, route: q.route || null, model: q.model || null };
}

/**
 * 拉报表：序号自增，过期响应丢弃；完成后 shouldRerender() 为真才重绘（日期框有焦点时不打断输入）。
 * dateEl：由日期框防抖提交触发时传入；返回时焦点仍在该框且值没再改过（用户停顿中），也重绘以显示结果。
 */
async function loadUsage(dateEl?: HTMLInputElement): Promise<void> {
  clearTimeout(usageTimer);
  usageTimer = 0;
  const seq = ++usageSeq;
  usageAt = Date.now();
  try {
    const r = await invoke<UsageReport>('usage_report', usageArgs());
    if (seq !== usageSeq) return;
    usage = r;
    usageErr = null;
  } catch (e) {
    if (seq !== usageSeq) return;
    usageErr = usageErrText(e);
  }
  if (!statsVisible()) return;
  const idle = !!dateEl && document.activeElement === dateEl && dateEl.value === usageQ[dateEl.id === 'u-from' ? 'from' : 'to'];
  if (shouldRerender() || idle) render();
}

/** 提交自定义日期：失焦 / 回车立即调用，change 停顿 USAGE_DATE_DEBOUNCE 后调用；不完整、年份 < 2000 或没变化都不查 */
function commitUsageDate(el: HTMLInputElement) {
  clearTimeout(usageDateTimer);
  usageDateTimer = 0;
  if (!okDate(el.value)) return;
  const key = el.id === 'u-from' ? 'from' : 'to';
  if (usageQ[key] === el.value) return;
  usageQ[key] = el.value;
  usageQ.bucket = null;
  void loadUsage(el);
}

/** activity 事件很频繁：统计页可见时节流刷新，两次请求至少间隔 USAGE_MIN_GAP */
function scheduleUsage() {
  if (!statsVisible() || usageTimer) return;
  usageTimer = window.setTimeout(() => { usageTimer = 0; void loadUsage(); }, Math.max(0, usageAt + USAGE_MIN_GAP - Date.now()));
}

/** 把多个桶里的同线路同模型行合并（未选桶时明细表显示整个范围） */
function mergeRows(rows: UsageModelRow[]): UsageModelRow[] {
  const m = new Map<string, UsageModelRow>();
  for (const r of rows) {
    const k = `${r.route}\u0000${r.model}`;
    const cur = m.get(k);
    if (!cur) { m.set(k, { ...r, totals: { ...r.totals } }); continue; }
    const a = cur.totals, b = r.totals;
    a.count += b.count; a.errors += b.errors; a.ms += b.ms; a.input += b.input; a.cacheRead += b.cacheRead; a.cacheWrite += b.cacheWrite; a.output += b.output; a.costUsd += b.costUsd;
  }
  return [...m.values()].sort((x, y) => y.totals.costUsd - x.totals.costUsd || tokAll(y.totals) - tokAll(x.totals) || x.model.localeCompare(y.model));
}

/** 线路占比条：按 pick 取值，三条线路固定顺序与颜色 */
function shareBar(by: Record<string, UsageTotals> | undefined, pick: (x: UsageTotals) => number, fmt: (v: number) => string) {
  const vals = ROUTES.map((r) => [r, by?.[r] ? pick(by[r]) : 0] as const);
  const sum = vals.reduce((a, [, v]) => a + v, 0);
  return `<div class="ushare">${sum > 0 ? vals.filter(([, v]) => v > 0).map(([r, v]) => `<i class="r-${r}" style="width:${(v / sum) * 100}%" title="${esc(`${routeName(r)} · ${fmt(v)}`)}"></i>`).join('') : ''}</div>`;
}

function statsView(_s: Status): string {
  const q = usageQ;
  const r = usage;
  const today = q.gran === 'today';
  const btn = (act: string, v: string, on: boolean, label: string) => `<button class="${on ? 'on' : ''}" data-act="${act}" data-v="${v}">${label}</button>`;
  const gran = `<div class="seg useg">${btn('usage-gran', 'today', today, t('usageToday'))}${btn('usage-gran', 'day', q.gran === 'day', t('usageDaily'))}${btn('usage-gran', 'month', q.gran === 'month', t('usageMonthly'))}</div>`;
  const ranges = today ? '' : `<div class="upills">${([7, 30, 90] as const).map((n) => btn('usage-range', String(n), q.range === n, t('usageLast', { n }))).join('')}${btn('usage-range', 'custom', q.range === 'custom', t('usageCustom'))}</div>`;
  const a = usageArgs();
  const max = ymd(new Date());
  const custom = !today && q.range === 'custom'
    ? `<div class="urow udates"><input type="date" class="v" id="u-from" value="${esc(a.from)}" max="${max}" aria-label="${esc(t('usageFrom'))}"><span>–</span><input type="date" class="v" id="u-to" value="${esc(a.to)}" max="${max}" aria-label="${esc(t('usageTo'))}"></div>` : '';
  // 下拉选项来自报表（不受过滤影响）；当前选中的值不在列表里也保留，避免选项消失
  const withCur = (list: string[], cur: string) => (cur && !list.includes(cur) ? [...list, cur] : list);
  const routeOpts = [{ value: '', label: t('usageAllRoutes') }, ...withCur(r?.routes ?? [], q.route).map((x) => ({ value: x, label: routeName(x) }))];
  const modelOpts = [{ value: '', label: t('usageAllModels') }, ...withCur(r?.models ?? [], q.model).map((x) => ({ value: x, label: x }))];
  const filters = `<div class="ufilters">
    <div class="urow">${gran}${ranges}</div>${custom}
    <div class="urow"><div class="usel">${renderSelect('u-route', routeOpts, q.route, t('usageAllRoutes'))}</div><div class="usel">${renderSelect('u-model', modelOpts, q.model, t('usageAllModels'))}</div>
      <button class="btn sm" data-act="usage-export">${icon.download(13)} ${t('usageExport')}</button></div></div>`;

  const notes = (usageErr ? `<div class="unote err">${icon.alert(15)}<span>${esc(usageErr)}</span></div>` : '')
    + (r?.unpricedModels.length ? `<div class="unote">${icon.alert(15)}<span>${esc(t('usageUnpriced', { models: r.unpricedModels.join(', ') }))}</span></div>` : '');

  const tot = r?.totals;
  const days = Math.max(1, r?.days ?? 1);
  const avg = (v: string) => (today ? '' : t('usageDailyAvg', { v }));
  const card = (k: string, v: string, sub: string, bar: string) => `<div class="ucard"><div class="k">${k}</div><div class="v">${v}</div><div class="sub">${sub || '&nbsp;'}</div>${bar}</div>`;
  const hr = tot ? hitRate(tot) : null;
  const cards = `<div class="ucards">
    ${card(`${esc(t('usageCost'))}<span class="utag">${esc(t('usageApiPrice'))}</span>`, tot ? fmtUsd(tot.costUsd) : '—', tot ? esc(avg(fmtUsd(tot.costUsd / days))) : '', shareBar(r?.byRoute, (x) => x.costUsd, fmtUsd))}
    ${card(esc(t('usageRequests')), tot ? tot.count.toLocaleString() : '—', tot ? esc(avg(fmtNum(tot.count / days))) : '', shareBar(r?.byRoute, (x) => x.count, (v) => v.toLocaleString()))}
    ${card(esc(t('usageTokens')), tot ? fmtTok(tokAll(tot)) : '—', tot ? esc(t('usageInOut', { in: fmtTok(tokIn(tot)), out: fmtTok(tot.output) })) : '', shareBar(r?.byRoute, tokAll, fmtTok))}
    ${card(esc(t('usageHitRate')), fmtPct(hr), tot && tokIn(tot) ? `${fmtTok(tot.cacheRead)} / ${fmtTok(tokIn(tot))}` : '', `<div class="ushare"><i class="hit" style="width:${(hr ?? 0) * 100}%"></i></div>`)}
  </div>`;
  const present = ROUTES.filter((x) => r?.byRoute[x]);
  const legend = present.length ? `<div class="ulegend">${present.map((x) => `<span><i class="rdot r-${x}"></i>${esc(routeName(x))}</span>`).join('')}</div>` : '';

  // 图表区（Task 6 在占位上挂 ECharts）；「今天」只有一个桶，不画图
  // 与 usage-chart.ts chartInput 的单位判断一致：范围内总金额为 0 时图表画 Tokens
  const priced = !!r && r.totals.costUsd !== 0;
  const chart = today ? '' : `<section class="card ucard-chart"><div class="sec-h"><b>${esc(priced ? `${t('usageCost')} · ${t('usageApiPrice')}` : t('usageTokensChart'))}</b></div><div id="usage-chart-slot" class="uchart"></div></section>`;

  // 明细表：选中桶 → 该桶各模型；未选 → 整个范围合并
  const bucket = q.bucket ? r?.buckets.find((b) => b.key === q.bucket) ?? null : null;
  const rows = bucket ? bucket.models : mergeRows((r?.buckets ?? []).flatMap((b) => b.models));
  const title = today ? t('usageToday') : bucket ? bucket.key : r ? (r.from === r.to ? r.from : `${r.from} – ${r.to}`) : '';
  const clear = bucket ? `<button class="link" data-act="usage-bucket" data-key="" aria-label="clear">${icon.close(13)}</button>` : '';
  const tr = (m: UsageModelRow) => `<tr><td class="um" title="${esc(`${routeName(m.route)} · ${m.model}`)}"><div><i class="rdot r-${esc(m.route)}"></i><span>${esc(m.model)}</span></div></td>
    <td>${m.totals.count.toLocaleString()}</td><td>${fmtTok(tokAll(m.totals))}</td><td>${fmtPct(hitRate(m.totals))}</td><td>${m.priced ? fmtUsd(m.totals.costUsd) : '—'}</td></tr>`;
  const table = !r ? '' : !rows.length ? `<section class="card"><div class="empty">${t('usageEmpty')}</div></section>`
    : `<section class="card utable-card"><div class="sec-h"><b>${esc(title)}</b>${clear}</div>
      <table class="utable"><colgroup><col><col class="n"><col class="n"><col class="n"><col class="n"></colgroup>
        <thead><tr><th>${t('usageColModel')}</th><th>${t('usageColReq')}</th><th>${t('usageColTokens')}</th><th>${t('usageColHit')}</th><th>${t('usageColCost')}</th></tr></thead>
        <tbody>${rows.map(tr).join('')}</tbody></table></section>`;
  return `${filters}${notes}${cards}${legend}${chart}${table}`;
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
type Check = 'wait' | 'run' | 'ok' | 'fail' | 'warn';
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
  cc = ob.env.claudeCode;
  for (let i = 1; i <= 5; i++) setTimeout(() => { ob.shown = i; if (obVisible()) render(); }, i * 220);
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

function obChecks(): Record<'cli' | 'login' | 'net' | 'cfg' | 'cc', Check> {
  const e = ob.env;
  const auth = status?.auth ?? e?.auth;
  const at = (i: number, v: Check): Check => (!e ? 'run' : ob.shown < i ? 'wait' : v);
  return {
    cli: at(1, e?.codexCli ? 'ok' : 'fail'),
    login: at(2, auth && auth.kind !== 'missing' ? 'ok' : 'fail'),
    net: at(3, ob.net === 'wait' || ob.net === 'run' ? 'run' : ob.net.ok ? 'ok' : 'fail'),
    cfg: at(4, e?.settingsOk ? 'ok' : 'fail'),
    // 版本不够只是选单里看不到 Codex 模型，代理照样能用，所以是黄色提示、不拦「一键接管」
    cc: at(5, e?.claudeCode.ok ? 'ok' : 'warn'),
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
    const ic = st === 'ok' ? icon.check(13) : st === 'fail' ? icon.close(13) : st === 'warn' ? '!' : st === 'run' ? '<span class="spinner"></span>' : '';
    const bad = st === 'fail' || st === 'warn';
    return `<div class="chk"><span class="st ${st}">${ic}</span><div class="tx"><b>${title}</b>
      <div class="d">${st === 'ok' ? okText : bad ? failText : st === 'run' ? t('obChecking') : t('obWaiting')}</div>${bad && fix ? `<div class="fix">${fix}</div>` : ''}</div></div>`;
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
    row(c.cc, t('ccTitle'), `v${esc(e?.claudeCode.version ?? '')}`,
      e?.claudeCode.version
        ? `${t('ccOldBody', { version: esc(e.claudeCode.version), min: esc(e.claudeCode.min) })} <code>${CC_UPDATE_CMD}</code>`
        : t('ccMissing', { min: esc(e?.claudeCode.min ?? '') }),
      e?.claudeCode.version ? `<button class="btn dark" data-act="copy-cc-update">${icon.copy(14)} ${t('copy')}</button>${recheck}` : recheck),
  ].join('');
  // Claude Code 版本只提示不拦：等它检测完即可
  const ready = (['cli', 'login', 'net', 'cfg'] as const).every((k) => c[k] === 'ok') && (c.cc === 'ok' || c.cc === 'warn');
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
        if (statsVisible()) void loadUsage();
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
      case 'copy-cc-update': await navigator.clipboard.writeText(CC_UPDATE_CMD); toast(t('toastCopiedCmd')); break;
      case 'cc-recheck':
        cc = await invoke<ClaudeCodeCheck>('claude_code_check');
        render();
        if (cc.ok) toast(t('ccOkToast', { version: cc.version ?? '' }));
        break;
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
      case 'feed-mode': feedMode = el.dataset.mode as typeof feedMode; render(); if (statsVisible()) void loadUsage(); break;
      // 统计页筛选：改条件后立即重新拉报表，清掉选中的桶
      case 'usage-gran': {
        const g = el.dataset.v as typeof usageQ.gran;
        if (g === usageQ.gran) break;
        usageQ.gran = g;
        if (g === 'month' && typeof usageQ.range === 'number' && usageQ.range < 90) usageQ.range = 90; // 按月看 7 / 30 天意义不大
        usageQ.bucket = null; render(); await loadUsage();
        break;
      }
      case 'usage-range': {
        const v = el.dataset.v === 'custom' ? 'custom' : (Number(el.dataset.v) as 7 | 30 | 90);
        if (v === usageQ.range) break;
        if (v === 'custom') { const a = usageArgs(); usageQ.from = a.from; usageQ.to = a.to; } // 自定义从当前范围起步
        usageQ.range = v; usageQ.bucket = null; render(); await loadUsage();
        break;
      }
      case 'usage-bucket': { const k = el.dataset.key || null; usageQ.bucket = k === usageQ.bucket ? null : k; render(); break; }
      case 'usage-export': {
        const path = await invoke<string>('export_usage_csv', usageArgs());
        toast(t('usageExported', { path }));
        break;
      }
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
  if (el.id === 'u-route' || el.id === 'u-model') {
    const v = selectValue(el.id);
    if (el.id === 'u-route') usageQ.route = v; else usageQ.model = v;
    usageQ.bucket = null;
    return void loadUsage();
  }
  // 自定义日期框：逐位输入时每段都会触发 change，这里只做防抖——值完整且年份 ≥ 2000 时停顿 700ms 再提交，
  // 期间再改就重新计时，中间值（0002、0020…）直接取消计时；失焦 / 回车仍立即提交（见 focusout、keydown）。
  // 日历弹窗选完日期焦点留在框里不会失焦，靠这里自动查询。
  if (el.id === 'u-from' || el.id === 'u-to') {
    clearTimeout(usageDateTimer);
    usageDateTimer = okDate(el.value) ? window.setTimeout(() => commitUsageDate(el), USAGE_DATE_DEBOUNCE) : 0;
    return;
  }
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
  if (e.key === 'Enter' && (el.dataset?.save || el.id === 'u-from' || el.id === 'u-to')) el.blur();
});
app.addEventListener('focusout', (e) => {
  const el = e.target as HTMLInputElement;
  if (el.id === 'u-from' || el.id === 'u-to') commitUsageDate(el);
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
  // 统计页不显示请求列表：不立即重绘，只节流刷新报表（报表回来再重绘）
  if (statsVisible()) return scheduleUsage();
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
  if (q.has('mock') && (q.get('feed') === 'raw' || q.get('feed') === 'req')) feedMode = q.get('feed') as 'raw' | 'req';
  if (q.has('mock') && q.has('sheet')) sheetOpen = true;
  // 统计页预览：ugran=today|month，urange=custom，uroute=线路
  if (q.has('mock') && (q.get('ugran') === 'today' || q.get('ugran') === 'month')) usageQ.gran = q.get('ugran') as 'today' | 'month';
  if (q.has('mock') && q.get('urange') === 'custom') { usageQ.range = 'custom'; usageQ.from = daysAgo(13); usageQ.to = ymd(new Date()); }
  if (q.has('mock') && q.get('uroute')) usageQ.route = q.get('uroute')!; // 如 codex：只看未知价格线路，图表改画 Tokens
  if (q.has('mock') && q.has('err')) seenErrAt = 0;
  if (q.has('mock') && q.has('capidel')) { const c = status.customApis[0]; if (c) capiDel = { id: c.id, model: c.model }; }
  const cp = q.has('mock') ? q.get('capi') : null; // 预览：progress = 校验中途，fail = 失败态，edit = 编辑态
  if (cp) {
    const c = status.customApis[cp === 'edit' ? 0 : 1];
    capi = { id: cp === 'edit' ? c.id : null, mask: c.keyMask, url: c.baseUrl, key: '', model: c.model, running: cp === 'progress', steps: {}, error: null, failed: cp === 'fail' };
    if (cp === 'progress') capi.steps = Object.fromEntries(mockStep(1, false)); else if (cp === 'fail') capi.steps = Object.fromEntries(mockStep(2, true));
  }
  render();
  if (statsVisible()) void loadUsage();
  if (status.onboarded) void invoke<ClaudeCodeCheck>('claude_code_check').then((r) => { cc = r; if (shouldRerender()) render(); }).catch(() => {});
  if (!status.onboarded) await loadEnv();
})();
