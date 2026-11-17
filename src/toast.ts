// 通知组件：从顶栏下方滑入的卡片，按类型显示图标；同一时间最多 3 条，新的在上面。
import { icon } from './icons';

export type ToastKind = 'success' | 'error' | 'info' | 'loading';

const DURATION: Record<ToastKind, number> = { success: 2200, info: 2600, loading: 2600, error: 5000 };
const MAX = 3;

function stack(): HTMLElement {
  let el = document.querySelector('.toasts') as HTMLElement | null;
  if (!el) {
    el = document.createElement('div');
    el.className = 'toasts';
    el.setAttribute('role', 'status');
    el.setAttribute('aria-live', 'polite');
    document.body.appendChild(el);
  }
  return el;
}

function dismiss(item: HTMLElement) {
  if (item.classList.contains('leaving')) return;
  item.classList.add('leaving');
  item.addEventListener('animationend', () => item.remove(), { once: true });
  setTimeout(() => item.remove(), 400);
}

export function toast(text: string, kind: ToastKind = 'success') {
  const host = stack();
  const ic = kind === 'success' ? icon.check(14) : kind === 'error' ? icon.alert(14) : kind === 'loading' ? '<span class="spinner"></span>' : icon.refresh(14);
  const item = document.createElement('div');
  item.className = `toast-item ${kind}`;
  item.innerHTML = `<span class="toast-ic">${ic}</span><span class="toast-text"></span>`;
  (item.querySelector('.toast-text') as HTMLElement).textContent = text;
  item.addEventListener('click', () => dismiss(item));
  host.prepend(item);
  [...host.children].slice(MAX).forEach((e) => dismiss(e as HTMLElement));
  setTimeout(() => dismiss(item), DURATION[kind]);
}
