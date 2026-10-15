import type { ITheme } from "@xterm/xterm";

export const cotermDarkTheme: ITheme = {
  background: "#020617", // slate-950
  foreground: "#f8fafc", // slate-50
  cursor: "#38bdf8",     // sky-400
  cursorAccent: "#020617",
  selectionBackground: "#33415580", // slate-700 with opacity
  selectionForeground: "#ffffff",

  // ANSI 16 Colors
  black: "#0f172a",
  red: "#f87171",
  green: "#4ade80",
  yellow: "#facc15",
  blue: "#60a5fa",
  magenta: "#c084fc", // Purple for Agent
  cyan: "#38bdf8",
  white: "#e2e8f0",

  brightBlack: "#475569",
  brightRed: "#ef4444",
  brightGreen: "#22c55e",
  brightYellow: "#eab308",
  brightBlue: "#3b82f6",
  brightMagenta: "#a855f7",
  brightCyan: "#0ea5e9",
  brightWhite: "#ffffff",
};
