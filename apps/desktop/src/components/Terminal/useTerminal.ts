import { useEffect, useRef, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { cotermDarkTheme } from "./terminalTheme";

interface UseTerminalOptions {
  onData?: (data: string) => void;
  onResize?: (cols: number, rows: number) => void;
  fontSize?: number;
  fontFamily?: string;
}

export function useTerminal(options: UseTerminalOptions = {}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  const {
    onData,
    onResize,
    fontSize = 13,
    fontFamily = "Cascadia Code, Fira Code, Consolas, Monaco, monospace",
  } = options;

  useEffect(() => {
    if (!containerRef.current) return;

    // 1. 创建 Terminal 实例
    const term = new Terminal({
      theme: cotermDarkTheme,
      fontSize,
      fontFamily,
      cursorBlink: true,
      cursorStyle: "bar",
      lineHeight: 1.25,
      allowTransparency: true,
      convertEol: true,
      scrollback: 10000,
      drawBoldTextInBrightColors: true,
    });

    // 2. 挂载 FitAddon
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    // 3. 打开至 DOM 容器
    term.open(containerRef.current);

    // 4. 尝试挂载 WebglAddon 加速渲染 (降级容错)
    try {
      const webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => {
        webglAddon.dispose();
      });
      term.loadAddon(webglAddon);
    } catch {
      console.warn("WebGL 不可用，降级使用 Canvas/DOM 渲染模式");
    }

    // 5. 首次适配尺寸
    requestAnimationFrame(() => {
      try {
        fitAddon.fit();
        onResize?.(term.cols, term.rows);
      } catch (e) {
        console.error("FitAddon error on init", e);
      }
    });

    // 6. 输入事件监听
    const disposableData = term.onData((data) => {
      onData?.(data);
    });

    // 7. 尺寸自适应监听
    const resizeObserver = new ResizeObserver(() => {
      requestAnimationFrame(() => {
        if (!containerRef.current || containerRef.current.clientWidth === 0) return;
        try {
          fitAddon.fit();
          onResize?.(term.cols, term.rows);
        } catch (e) {
          console.error("FitAddon error on resize", e);
        }
      });
    });

    resizeObserver.observe(containerRef.current);

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    return () => {
      disposableData.dispose();
      resizeObserver.disconnect();
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, [fontSize, fontFamily]);

  const write = useCallback((data: string) => {
    terminalRef.current?.write(data);
  }, []);

  const clear = useCallback(() => {
    terminalRef.current?.clear();
  }, []);

  const focus = useCallback(() => {
    terminalRef.current?.focus();
  }, []);

  return {
    containerRef,
    terminal: terminalRef.current,
    write,
    clear,
    focus,
  };
}
