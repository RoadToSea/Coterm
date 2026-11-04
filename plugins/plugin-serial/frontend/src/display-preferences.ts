import { readReceiveNewlineMode, RECEIVE_NEWLINE_MODES, type ReceiveNewlineMode } from "./receive-newlines";

export interface SerialDisplayPreferences {
  view: "terminal" | "uart";
  receiveFormat: "text" | "hex";
  timestamps: boolean;
  autoScroll: boolean;
  receiveNewlines: ReceiveNewlineMode;
  backspace: "bs" | "del";
}

export const DEFAULT_SERIAL_DISPLAY_PREFERENCES: Readonly<SerialDisplayPreferences> = Object.freeze({
  view: "terminal", receiveFormat: "text", timestamps: false, autoScroll: true,
  receiveNewlines: "none", backspace: "bs",
});
const PREFIX = "coterm.serial.display.v1:";
const MAX_CHARS = 2048;

/** Never identify a saved instance by device address or display name. */
function storageKey(configRef?: string, runtimeInstanceId?: string): string | null {
  const value = configRef ?? runtimeInstanceId;
  if (typeof value !== "string") return null;
  if (!value || value.length > 512 || /[\u0000-\u001f\u007f]/.test(value)) return null;
  if (!value.trim()) return null;
  return PREFIX + (configRef === undefined ? "runtime:" : "config:") + encodeURIComponent(value);
}

function normalize(value: Record<string, unknown>): SerialDisplayPreferences {
  return {
    view: value.view === "uart" ? "uart" : "terminal",
    receiveFormat: value.receiveFormat === "hex" ? "hex" : "text",
    timestamps: typeof value.timestamps === "boolean" ? value.timestamps : false,
    autoScroll: typeof value.autoScroll === "boolean" ? value.autoScroll : true,
    receiveNewlines: RECEIVE_NEWLINE_MODES.find(mode => mode === value.receiveNewlines) ?? "none",
    backspace: value.backspace === "del" ? "del" : "bs",
  };
}

export function readSerialDisplayPreferences(configRef?: string, runtimeInstanceId?: string): SerialDisplayPreferences {
  const defaults = { ...DEFAULT_SERIAL_DISPLAY_PREFERENCES };
  try {
    const key = storageKey(configRef, runtimeInstanceId);
    if (!key) return defaults;
    const raw = localStorage.getItem(key);
    if (raw !== null) {
      if (raw.length > MAX_CHARS) return defaults;
      const value: unknown = JSON.parse(raw);
      if (!value || typeof value !== "object" || Array.isArray(value) || !("version" in value) || value.version !== 1) return defaults;
      return normalize(value as Record<string, unknown>);
    }
    // Migrate only the current runtime instance; never guess another device's legacy settings.
    if (runtimeInstanceId) {
      defaults.receiveNewlines = readReceiveNewlineMode(runtimeInstanceId);
      if (defaults.receiveNewlines !== "none") saveSerialDisplayPreferences(configRef, defaults, runtimeInstanceId);
    }
  } catch { /* Invalid or unavailable storage leaves a usable current view. */ }
  return defaults;
}

/** Only bounded UI choices are saved, never commands, output, files or connection credentials. */
export function saveSerialDisplayPreferences(configRef: string | undefined, preferences: SerialDisplayPreferences,
  runtimeInstanceId?: string): void {
  try {
    const key = storageKey(configRef, runtimeInstanceId);
    if (key) localStorage.setItem(key, JSON.stringify({ version: 1, ...normalize({ ...preferences }) }));
  } catch { /* Changes still apply to the current view when storage is unavailable. */ }
}
