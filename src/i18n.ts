// Internationalization (i18n): RU/EN dictionaries, persisted language choice
// and live switching without a reload. All user-facing strings must go through
// t(); static markup is translated via data-i18n* attributes.

import en from './locales/en.json';
import ru from './locales/ru.json';
import { api } from './api';

export type Language = 'en' | 'ru';

const DICTS: Record<Language, Record<string, string>> = { en, ru };
const STORAGE_KEY = 'discord-client.lang';
const CHANGE_EVENT = 'dc:languagechange';

let current: Language = 'en';

export function getLanguage(): Language {
  return current;
}

/** Translate a key, interpolating {name} placeholders from `vars`. */
export function t(key: string, vars?: Record<string, string | number>): string {
  const raw = DICTS[current][key] ?? DICTS.en[key] ?? key;
  if (!vars) return raw;
  return raw.replace(/\{(\w+)\}/g, (_, name: string) =>
    vars[name] !== undefined ? String(vars[name]) : `{${name}}`,
  );
}

function isLanguage(value: string | null): value is Language {
  return value === 'en' || value === 'ru';
}

/** Translate data-i18n, data-i18n-html, data-i18n-placeholder and data-i18n-title nodes. */
function applyStaticTexts(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>('[data-i18n]').forEach((node) => {
    node.textContent = t(node.dataset.i18n ?? '');
  });
  root.querySelectorAll<HTMLElement>('[data-i18n-html]').forEach((node) => {
    node.innerHTML = t(node.dataset.i18nHtml ?? '');
  });
  root.querySelectorAll<HTMLElement>('[data-i18n-placeholder]').forEach((node) => {
    node.setAttribute('placeholder', t(node.dataset.i18nPlaceholder ?? ''));
  });
  root.querySelectorAll<HTMLElement>('[data-i18n-title]').forEach((node) => {
    node.setAttribute('title', t(node.dataset.i18nTitle ?? ''));
  });
  root.querySelectorAll<HTMLElement>('[data-i18n-aria-label]').forEach((node) => {
    node.setAttribute('aria-label', t(node.dataset.i18nAriaLabel ?? ''));
  });
}

function syncSelects(): void {
  document.querySelectorAll<HTMLSelectElement>('[data-language-select]').forEach((select) => {
    select.value = current;
  });
}

/**
 * Switch the UI language: updates static markup, persisted choice, the backend
 * (for the few user-facing error strings it owns) and emits a change event so
 * dynamic widgets can re-render.
 */
export function applyLanguage(lang: Language, persist = true): void {
  current = lang;
  document.documentElement.lang = lang;
  if (persist) {
    try {
      localStorage.setItem(STORAGE_KEY, lang);
    } catch {
      // Storage may be unavailable (private mode) — the choice is still applied.
    }
  }
  applyStaticTexts();
  syncSelects();
  void api.setLanguage(lang).catch(() => {});
  window.dispatchEvent(new CustomEvent<Language>(CHANGE_EVENT, { detail: lang }));
}

/** Detect the initial language (stored choice → system locale) and bind selects. */
export function initI18n(): void {
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(STORAGE_KEY);
  } catch {
    // ignore
  }
  const systemIsRussian = (navigator.language || '').toLowerCase().startsWith('ru');
  const initial: Language = isLanguage(stored) ? stored : systemIsRussian ? 'ru' : 'en';

  applyLanguage(initial, false);

  document.querySelectorAll<HTMLSelectElement>('[data-language-select]').forEach((select) => {
    select.value = initial;
    select.addEventListener('change', () => {
      const next = select.value;
      if (isLanguage(next)) applyLanguage(next);
    });
  });
}

/** Subscribe to language changes (for re-rendering dynamic widgets). */
export function onLanguageChange(cb: (lang: Language) => void): void {
  window.addEventListener(CHANGE_EVENT, () => cb(current));
}
