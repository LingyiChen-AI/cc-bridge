// 界面多语言。后端返回错误码 / 原始详情，这里负责翻译。新增文案先加到 locales/zh.ts（基准）。
import { zh, type Dict } from './locales/zh';
import { en } from './locales/en';
import { zhTW } from './locales/zh-TW';
import { ja } from './locales/ja';
import { ko } from './locales/ko';
import { es } from './locales/es';
import { fr } from './locales/fr';
import { de } from './locales/de';
import { ru } from './locales/ru';
import { pt } from './locales/pt';

export type Key = keyof Dict;

/** 支持的语言：code 与后端 AppConfig.language / StatusDto.lang 一致；name 用该语言自己的写法 */
export const LANGS = [
  { code: 'zh', name: '简体中文', htmlLang: 'zh-CN', dict: zh },
  { code: 'zh-TW', name: '繁體中文', htmlLang: 'zh-TW', dict: zhTW },
  { code: 'en', name: 'English', htmlLang: 'en', dict: en },
  { code: 'ja', name: '日本語', htmlLang: 'ja', dict: ja },
  { code: 'ko', name: '한국어', htmlLang: 'ko', dict: ko },
  { code: 'es', name: 'Español', htmlLang: 'es', dict: es },
  { code: 'fr', name: 'Français', htmlLang: 'fr', dict: fr },
  { code: 'de', name: 'Deutsch', htmlLang: 'de', dict: de },
  { code: 'ru', name: 'Русский', htmlLang: 'ru', dict: ru },
  { code: 'pt', name: 'Português', htmlLang: 'pt', dict: pt },
] as const;

export type Lang = (typeof LANGS)[number]['code'];

/** 浏览器语言 → 支持的语言（后端没给 lang 之前用） */
function detect(): Lang {
  const l = navigator.language.toLowerCase();
  if (l.startsWith('zh')) return /tw|hk|mo|hant/.test(l) ? 'zh-TW' : 'zh';
  return (LANGS.find((x) => x.code === l.slice(0, 2))?.code ?? 'en') as Lang;
}

let current = LANGS.find((x) => x.code === detect())!;

export function setLang(code: string | undefined | null) {
  const found = LANGS.find((x) => x.code === code);
  if (found) {
    current = found;
    document.documentElement.lang = found.htmlLang;
  }
}

export function t(key: Key, vars: Record<string, string | number> = {}): string {
  const text = (current.dict as Dict)[key] ?? en[key] ?? key;
  return text.replace(/\{(\w+)\}/g, (_, k) => String(vars[k] ?? ''));
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
