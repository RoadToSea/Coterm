export const PREFERENCES_STORAGE_KEY = "coterm.preferences.v1";

export interface Preferences {
  interfaceFontSize: number;
  terminalFontSize: number;
  terminalLineHeight: number;
  cursorBlink: boolean;
}

export const DEFAULT_PREFERENCES: Readonly<Preferences> = Object.freeze({
  interfaceFontSize: 14,
  terminalFontSize: 13,
  terminalLineHeight: 1,
  cursorBlink: true,
});

export function resolvePreferences(value: unknown): Preferences {
  const saved = typeof value === "object" && value !== null ? value : {};
  const number = (key: "interfaceFontSize" | "terminalFontSize" | "terminalLineHeight", min: number, max: number, integer = false) => {
    const candidate = key in saved ? Reflect.get(saved, key) : undefined;
    return typeof candidate === "number" && Number.isFinite(candidate) && candidate >= min && candidate <= max
      && (!integer || Number.isInteger(candidate)) ? candidate : DEFAULT_PREFERENCES[key];
  };
  const blink = "cursorBlink" in saved ? Reflect.get(saved, "cursorBlink") : undefined;
  return {
    interfaceFontSize: number("interfaceFontSize", 12, 20, true),
    terminalFontSize: number("terminalFontSize", 10, 28, true),
    terminalLineHeight: number("terminalLineHeight", 1, 2),
    cursorBlink: typeof blink === "boolean" ? blink : DEFAULT_PREFERENCES.cursorBlink,
  };
}

function readPreferences(): Preferences {
  try { return resolvePreferences(JSON.parse(globalThis.localStorage?.getItem(PREFERENCES_STORAGE_KEY) ?? "null")); }
  catch { return { ...DEFAULT_PREFERENCES }; }
}

let preferences = readPreferences();
const listeners = new Set<() => void>();
export const getPreferences = () => preferences;
export function subscribePreferences(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function applyPreferences(): void {
  if (typeof document === "undefined") return;
  const style = document.documentElement.style;
  style.setProperty("--coterm-ui-font-size", `${preferences.interfaceFontSize}px`);
  style.setProperty("--coterm-root-font-size", `${preferences.interfaceFontSize / 14 * 16}px`);
  style.setProperty("--coterm-terminal-font-size", `${preferences.terminalFontSize}px`);
  style.setProperty("--coterm-terminal-line-height", String(preferences.terminalLineHeight));
  style.setProperty("--coterm-terminal-cursor-blink", String(preferences.cursorBlink));
}

export function setPreferences(patch: Partial<Preferences>): void {
  const next = resolvePreferences({ ...preferences, ...patch });
  try { globalThis.localStorage?.setItem(PREFERENCES_STORAGE_KEY, JSON.stringify(next)); } catch { /* Keep session preferences. */ }
  if (JSON.stringify(next) === JSON.stringify(preferences)) return;
  preferences = next;
  applyPreferences();
  for (const listener of listeners) listener();
}

if (typeof window !== "undefined") window.addEventListener("storage", (event) => {
  if (event.key !== PREFERENCES_STORAGE_KEY && event.key !== null) return;
  preferences = readPreferences();
  applyPreferences();
  for (const listener of listeners) listener();
});
applyPreferences();
