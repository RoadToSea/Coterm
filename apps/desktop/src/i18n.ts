import { zhCN } from "./locales/zh-CN";
import { en } from "./locales/en";

export type Locale = "en" | "zh-CN";
export const LOCALE_STORAGE_KEY = "coterm.locale";
const listeners = new Set<() => void>();

export function resolveLocale(saved: string | null, languages: readonly string[]): Locale {
  if (saved === "en" || saved === "zh-CN") return saved;
  return languages[0]?.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

function initialLocale(): Locale {
  let saved: string | null = null;
  try { saved = globalThis.localStorage?.getItem(LOCALE_STORAGE_KEY) ?? null; } catch { /* Storage may be disabled. */ }
  return resolveLocale(saved, typeof navigator === "undefined" ? [] : navigator.languages);
}

let locale = initialLocale();
export const getLocale = (): Locale => locale;
export function subscribeLocale(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function setLocale(next: Locale): void {
  if (next !== "en" && next !== "zh-CN") return;
  try { globalThis.localStorage?.setItem(LOCALE_STORAGE_KEY, next); } catch { /* Keep the session preference. */ }
  if (locale === next) return;
  locale = next;
  if (typeof document !== "undefined") document.documentElement.lang = next;
  for (const listener of [...listeners]) {
    try { listener(); } catch { /* A plugin listener cannot interrupt other language updates. */ }
  }
}

/** English is the source language and the fallback for unknown plugin text. */
export function t(message: string, values: Record<string, string | number> = {}): string {
  const template = locale === "zh-CN" ? zhCN[message] ?? en[message] ?? message : en[message] ?? message;
  return template.replace(/\{(\w+)\}/g, (match, key: string) => String(values[key] ?? match));
}

/** Optional SDK capability; changing language never remounts a live connection. */
export const pluginI18n = Object.freeze({ get locale() { return locale; }, subscribe: subscribeLocale });
if (typeof document !== "undefined") document.documentElement.lang = locale;
