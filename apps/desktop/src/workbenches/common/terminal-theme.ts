/** Read the central palette; missing optional ANSI tokens are left to xterm. */
export function readTerminalTheme(element: HTMLElement) {
  const style = element.ownerDocument.defaultView?.getComputedStyle(element);
  const token = (name: string) => style?.getPropertyValue(`--terminal-${name}`).trim() || undefined;
  return {
    background: token("background"),
    foreground: token("foreground"),
    cursor: token("foreground"),
    cursorAccent: token("background"),
    selectionBackground: token("selection"),
    selectionInactiveBackground: token("selection"),
    black: token("black"),
    red: token("red"),
    green: token("green"),
    yellow: token("yellow"),
    blue: token("blue"),
    magenta: token("magenta"),
    cyan: token("cyan"),
    white: token("foreground"),
    brightBlack: token("bright-black"),
    brightRed: token("red"),
    brightGreen: token("green"),
    brightYellow: token("yellow"),
    brightBlue: token("blue"),
    brightMagenta: token("magenta"),
    brightCyan: token("cyan"),
    brightWhite: token("foreground"),
  };
}

/** After readTerminalTheme initializes options, subscribe to palette changes in place. */
export function attachTerminalTheme(element: HTMLElement, apply: (theme: ReturnType<typeof readTerminalTheme>) => void): () => void {
  let previous = JSON.stringify(readTerminalTheme(element));
  const update = () => {
    const theme = readTerminalTheme(element);
    const next = JSON.stringify(theme);
    if (next === previous) return;
    previous = next;
    apply(theme);
  };
  const observer = new MutationObserver(update);
  observer.observe(element.ownerDocument.documentElement, { attributes: true, attributeFilter: ["class", "data-theme", "style"] });
  return () => observer.disconnect();
}

/** Host typography is optional; standalone drivers keep their terminal defaults. */
export function readTerminalPreferences(element: HTMLElement) {
  const style = element.ownerDocument.defaultView?.getComputedStyle(element);
  const bounded = (name: string, fallback: number, min: number, max: number) => {
    const value = Number.parseFloat(style?.getPropertyValue(name) ?? "");
    return Number.isFinite(value) && value >= min && value <= max ? value : fallback;
  };
  return {
    fontSize: bounded("--coterm-terminal-font-size", 13, 10, 28),
    lineHeight: bounded("--coterm-terminal-line-height", 1, 1, 2),
    cursorBlink: style?.getPropertyValue("--coterm-terminal-cursor-blink").trim() !== "false",
  };
}

/** Update options and fit the existing renderer; never recreate a live terminal. */
export function attachTerminalPreferences(element: HTMLElement, apply: (preferences: ReturnType<typeof readTerminalPreferences>) => void): () => void {
  let previous = JSON.stringify(readTerminalPreferences(element));
  const observer = new MutationObserver(() => {
    const preferences = readTerminalPreferences(element);
    const next = JSON.stringify(preferences);
    if (next === previous) return;
    previous = next;
    apply(preferences);
  });
  observer.observe(element.ownerDocument.documentElement, { attributes: true, attributeFilter: ["style"] });
  return () => observer.disconnect();
}
