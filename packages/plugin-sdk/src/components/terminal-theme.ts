/** CSS-only capability: older hosts without theme tokens retain the dark fallback. */
export function readTerminalTheme(element: HTMLElement) {
  const style = element.ownerDocument.defaultView?.getComputedStyle(element);
  const token = (name: string, fallback: string) => style?.getPropertyValue(`--terminal-${name}`).trim() || fallback;
  return {
    background: token("background", "#1e1e1e"),
    foreground: token("foreground", "#cccccc"),
    cursor: token("foreground", "#cccccc"),
    cursorAccent: token("background", "#1e1e1e"),
    selectionBackground: token("selection", "#334e70"),
    selectionInactiveBackground: token("selection", "#334e70"),
    black: token("black", "#171d27"),
    red: token("red", "#f58c86"),
    green: token("green", "#85d6a1"),
    yellow: token("yellow", "#e5c477"),
    blue: token("blue", "#80b7ff"),
    magenta: token("magenta", "#c79ae8"),
    cyan: token("cyan", "#77cedb"),
    white: token("foreground", "#cccccc"),
    brightBlack: token("bright-black", "#8996a8"),
    brightRed: token("red", "#f58c86"),
    brightGreen: token("green", "#85d6a1"),
    brightYellow: token("yellow", "#e5c477"),
    brightBlue: token("blue", "#80b7ff"),
    brightMagenta: token("magenta", "#c79ae8"),
    brightCyan: token("cyan", "#77cedb"),
    brightWhite: token("foreground", "#cccccc"),
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
