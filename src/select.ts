// 自绘下拉选择组件（替代原生 <select>）：鼠标 + 键盘（↑↓ / Enter / Esc / Home / End）、点外部关闭、空间不够自动向上弹出。
// 用法：renderSelect(...) 生成 HTML；installSelects() 只需调用一次（事件委托）；值变化时在 .sel 上派发冒泡的 'change' 事件。
import { icon } from './icons';

export interface SelectOption {
  value: string;
  label: string;
  hint?: string;
}

const esc = (s: unknown) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);

export function renderSelect(id: string, options: SelectOption[], value: string, ariaLabel: string): string {
  const cur = options.find((o) => o.value === value) ?? options[0];
  return `
    <div class="sel" id="${id}" data-value="${esc(cur.value)}">
      <button type="button" class="sel-btn input" aria-haspopup="listbox" aria-expanded="false" aria-label="${esc(ariaLabel)}">
        <span class="sel-label">${esc(cur.label)}</span><span class="sel-chev">${icon.chevron(14)}</span>
      </button>
      <div class="sel-pop" role="listbox" tabindex="-1">
        ${options
          .map(
            (o) => `<div class="sel-opt ${o.value === cur.value ? 'sel-on' : ''}" role="option" data-v="${esc(o.value)}" aria-selected="${o.value === cur.value}">
              <span class="sel-check">${icon.check(13)}</span><span class="sel-text">${esc(o.label)}</span>${o.hint ? `<span class="sel-hint">${esc(o.hint)}</span>` : ''}</div>`,
          )
          .join('')}
      </div>
    </div>`;
}

export const selectValue = (id: string) => (document.getElementById(id) as HTMLElement | null)?.dataset.value ?? '';
export const anySelectOpen = () => !!document.querySelector('.sel.open');

function open(sel: HTMLElement) {
  closeAll(sel);
  sel.classList.add('open');
  sel.querySelector('.sel-btn')!.setAttribute('aria-expanded', 'true');
  const pop = sel.querySelector('.sel-pop') as HTMLElement;
  const rect = sel.getBoundingClientRect();
  sel.classList.toggle('up', rect.bottom + Math.min(pop.scrollHeight, 280) + 8 > window.innerHeight && rect.top > 300);
  setActive(sel, sel.querySelector('.sel-on') as HTMLElement | null);
}

function close(sel: HTMLElement, focusButton = false) {
  sel.classList.remove('open', 'up');
  sel.querySelector('.sel-btn')!.setAttribute('aria-expanded', 'false');
  sel.querySelectorAll('.sel-active').forEach((e) => e.classList.remove('sel-active'));
  if (focusButton) (sel.querySelector('.sel-btn') as HTMLElement).focus();
}

function closeAll(except?: HTMLElement) {
  document.querySelectorAll<HTMLElement>('.sel.open').forEach((s) => s !== except && close(s));
}

function setActive(sel: HTMLElement, opt: HTMLElement | null) {
  sel.querySelectorAll('.sel-active').forEach((e) => e.classList.remove('sel-active'));
  if (opt) {
    opt.classList.add('sel-active');
    opt.scrollIntoView({ block: 'nearest' });
  }
}

function choose(sel: HTMLElement, opt: HTMLElement) {
  const v = opt.dataset.v ?? '';
  sel.querySelectorAll('.sel-opt').forEach((o) => {
    o.classList.toggle('sel-on', o === opt);
    o.setAttribute('aria-selected', String(o === opt));
  });
  (sel.querySelector('.sel-label') as HTMLElement).textContent = opt.querySelector('.sel-text')!.textContent;
  const changed = sel.dataset.value !== v;
  sel.dataset.value = v;
  close(sel, true);
  if (changed) sel.dispatchEvent(new Event('change', { bubbles: true }));
}

let installed = false;
export function installSelects() {
  if (installed) return;
  installed = true;
  document.addEventListener('click', (e) => {
    const target = e.target as HTMLElement;
    const sel = target.closest('.sel') as HTMLElement | null;
    if (!sel) return closeAll();
    const opt = target.closest('.sel-opt') as HTMLElement | null;
    if (opt) return choose(sel, opt);
    if (target.closest('.sel-btn')) sel.classList.contains('open') ? close(sel) : open(sel);
  });
  document.addEventListener('mouseover', (e) => {
    const opt = (e.target as HTMLElement).closest('.sel-opt') as HTMLElement | null;
    if (opt) setActive(opt.closest('.sel') as HTMLElement, opt);
  });
  document.addEventListener('keydown', (e) => {
    const sel = (e.target as HTMLElement).closest?.('.sel') as HTMLElement | null;
    if (!sel) return;
    const opts = [...sel.querySelectorAll<HTMLElement>('.sel-opt')];
    const active = sel.querySelector('.sel-active') as HTMLElement | null;
    const idx = active ? opts.indexOf(active) : opts.findIndex((o) => o.classList.contains('sel-on'));
    const isOpen = sel.classList.contains('open');
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp': {
        e.preventDefault();
        if (!isOpen) return open(sel);
        const next = e.key === 'ArrowDown' ? Math.min(opts.length - 1, idx + 1) : Math.max(0, idx - 1);
        setActive(sel, opts[next]);
        break;
      }
      case 'Home':
      case 'End':
        if (isOpen) {
          e.preventDefault();
          setActive(sel, e.key === 'Home' ? opts[0] : opts[opts.length - 1]);
        }
        break;
      case 'Enter':
      case ' ':
        e.preventDefault();
        if (isOpen && active) choose(sel, active);
        else if (!isOpen) open(sel);
        break;
      case 'Escape':
      case 'Tab':
        if (isOpen) {
          if (e.key === 'Escape') e.preventDefault();
          close(sel, e.key === 'Escape');
        }
        break;
    }
  });
  window.addEventListener('blur', () => closeAll());
}
