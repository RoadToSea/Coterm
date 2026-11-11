export const RECEIVE_NEWLINE_MODES = ["none", "lf", "cr", "both"] as const;
export type ReceiveNewlineMode = typeof RECEIVE_NEWLINE_MODES[number];

const KEY = "coterm.serial.receive-newlines.v1:";

export function readReceiveNewlineMode(instanceId: string): ReceiveNewlineMode {
  try {
    const value = localStorage.getItem(KEY + instanceId);
    return RECEIVE_NEWLINE_MODES.find(mode => mode === value) ?? "none";
  } catch { return "none"; }
}

export function saveReceiveNewlineMode(instanceId: string, mode: ReceiveNewlineMode): void {
  try { localStorage.setItem(KEY + instanceId, mode); } catch { /* The current view still works without storage. */ }
}

/** Display-only conversion. Carry at most two flags across chunks; never wait for another byte. */
export function createReceiveNewlineNormalizer(mode: ReceiveNewlineMode) {
  let previousCR = false;
  let insertedLF = false;
  const completeLF = mode === "lf" || mode === "both";
  const completeCR = mode === "cr" || mode === "both";
  return {
    reset() { previousCR = false; insertedLF = false; },
    write(text: string): string {
      if (mode === "none") return text;
      let result = "";
      for (const character of text) {
        if (character === "\n") {
          // A CR may already have supplied this LF, including in the preceding chunk.
          if (!insertedLF) result += completeLF && !previousCR ? "\r\n" : "\n";
        } else if (character === "\r") {
          result += completeCR ? "\r\n" : "\r";
        } else {
          result += character;
        }
        previousCR = character === "\r";
        insertedLF = previousCR && completeCR;
      }
      return result;
    },
  };
}
