export type ThemePreference = "light" | "dark" | "system";
export const THEME_STORAGE_KEY = "coterm.theme";

export function resolveThemePreference(value: string | null): ThemePreference {
  return value === "light" || value === "dark" ? value : "system";
}

export function resolveTheme(preference: ThemePreference, systemDark: boolean): "light" | "dark" {
  return preference === "system" ? systemDark ? "dark" : "light" : preference;
}

function readPreference(): ThemePreference {
  try { return resolveThemePreference(globalThis.localStorage?.getItem(THEME_STORAGE_KEY) ?? null); }
  catch { return "system"; }
}

let preference = readPreference();
const listeners = new Set<() => void>();
const media = typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)") : undefined;
export const getThemePreference = () => preference;
export function subscribeTheme(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function applyTheme(): void {
  if (typeof document === "undefined") return;
  const theme = resolveTheme(preference, media?.matches ?? false);
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
}

export function setThemePreference(next: ThemePreference): void {
  if (next !== "light" && next !== "dark" && next !== "system") return;
  try { globalThis.localStorage?.setItem(THEME_STORAGE_KEY, next); } catch { /* Retain the session preference. */ }
  if (next === preference) return;
  preference = next;
  applyTheme();
  for (const listener of listeners) listener();
}

media?.addEventListener("change", () => { if (preference === "system") applyTheme(); });
if (typeof window !== "undefined") window.addEventListener("storage", (event) => {
  if (event.key !== THEME_STORAGE_KEY && event.key !== null) return;
  preference = resolveThemePreference(event.newValue);
  applyTheme();
  for (const listener of listeners) listener();
});
applyTheme();
