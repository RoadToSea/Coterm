import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { Globe2, Palette, Plug, RotateCcw, Search, Settings2, TerminalSquare, X } from "lucide-react";
import { useI18n } from "../../use-i18n";
import { DEFAULT_PREFERENCES, getPreferences, setPreferences, subscribePreferences } from "../../preferences";
import { ThemeSwitcher } from "../ThemeSwitcher";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "../ui/empty";
import { cn } from "../../lib/utils";
import { McpPortSetting } from "./McpPortSetting";
import { McpClientConfigurationSetting, McpConnectionSetting } from "./McpConnectionSetting";
import { useMcpSettings } from "./use-mcp-settings";
import { Badge } from "../ui/badge";
import { Alert, AlertDescription } from "../ui/alert";

type Category = "general" | "appearance" | "terminal" | "mcp";
const categories = [
  { id: "all", label: "All settings", icon: Settings2 },
  { id: "general", label: "General", icon: Globe2 },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "terminal", label: "Terminal", icon: TerminalSquare },
  { id: "mcp", label: "MCP", icon: Plug },
] as const;

const controlClass = "h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring sm:w-44";

export function AppSettings({ onClose }: { onClose: () => void }) {
  const { t, locale, setLocale } = useI18n();
  const mcp = useMcpSettings();
  const preferences = useSyncExternalStore(subscribePreferences, getPreferences, getPreferences);
  const [category, setCategory] = useState<Category | "all">("all");
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => { searchRef.current?.focus(); }, []);
  const clearSearch = () => { setQuery(""); searchRef.current?.focus(); };
  const definitions: { id: string; category: Category; title: string; description?: string; keywords: string; control: ReactNode }[] = [
    { id: "language", category: "general", title: "Language", description: "Choose the language used by the app and supported plugins.", keywords: "locale translation 中文 简体中文 English en zh-CN",
      control: <select id="locale-select" className={controlClass} aria-label={t("Language")} value={locale} onChange={event => {
        if (event.target.value === "en" || event.target.value === "zh-CN") setLocale(event.target.value);
      }}><option value="zh-CN">简体中文</option><option value="en">English</option></select> },
    { id: "theme", category: "appearance", title: "Color theme", description: "Use a light theme, a dark theme, or follow your system.", keywords: "light dark system 主题 浅色 深色 系统",
      control: <ThemeSwitcher id="settings-theme-switcher" /> },
    { id: "interface-font-size", category: "appearance", title: "Interface font size", description: "Adjust app text and controls. Default: 14 px.", keywords: "font size text zoom 字体 字号 文字 缩放 px",
      control: <select id="interface-font-size" className={controlClass} aria-label={t("Interface font size")} value={preferences.interfaceFontSize} onChange={event => setPreferences({ interfaceFontSize: Number(event.target.value) })}>
        {Array.from({ length: 9 }, (_, index) => index + 12).map(size => <option key={size} value={size}>{size} px</option>)}
      </select> },
    { id: "terminal-font-size", category: "terminal", title: "Terminal font size", description: "Adjust terminal and command output text. Default: 13 px.", keywords: "font size output ssh serial 字体 字号 输出 串口 px",
      control: <select id="terminal-font-size" className={controlClass} aria-label={t("Terminal font size")} value={preferences.terminalFontSize} onChange={event => setPreferences({ terminalFontSize: Number(event.target.value) })}>
        {Array.from({ length: 19 }, (_, index) => index + 10).map(size => <option key={size} value={size}>{size} px</option>)}
      </select> },
    { id: "terminal-line-height", category: "terminal", title: "Terminal line height", description: "Set the space between terminal lines. Default: 1.0.", keywords: "line spacing 行距 行高",
      control: <select id="terminal-line-height" className={controlClass} aria-label={t("Terminal line height")} value={preferences.terminalLineHeight} onChange={event => setPreferences({ terminalLineHeight: Number(event.target.value) })}>
        {[1, 1.2, 1.4, 1.6, 1.8, 2].map(height => <option key={height} value={height}>{height.toFixed(1)}</option>)}
      </select> },
    { id: "cursor-blink", category: "terminal", title: "Blinking cursor", description: "Blink the cursor in terminal views.", keywords: "caret cursor blink 光标 闪烁",
      control: <input id="cursor-blink" type="checkbox" role="switch" className="settings-switch" aria-label={t("Blinking cursor")} checked={preferences.cursorBlink} onChange={event => setPreferences({ cursorBlink: event.target.checked })} /> },
    { id: "mcp-port", category: "mcp", title: "MCP port", keywords: "MCP Stdio bridge interface port localhost 端口 接口 本机 桥接",
      control: <McpPortSetting mcp={mcp} /> },
    { id: "mcp-connection", category: "mcp", title: "MCP connection", keywords: "MCP Stdio auto manual startup connection client 自启动 手动启动 开关 连接 客户端",
      control: <McpConnectionSetting mcp={mcp} /> },
    { id: "mcp-client-configuration", category: "mcp", title: "Client configuration", keywords: "MCP Stdio client configuration copy agent 客户端 配置 复制",
      control: <McpClientConfigurationSetting mcp={mcp} /> },
  ];
  const words = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  // Searching covers all categories so users never have to guess where a setting lives.
  const visible = definitions.filter(setting => {
    if (!words.length) return category === "all" || setting.category === category;
    const categoryLabel = categories.find(item => item.id === setting.category)!.label;
    const text = [setting.title, t(setting.title), setting.description, setting.description ? t(setting.description) : "", categoryLabel, t(categoryLabel), setting.keywords].join(" ").toLocaleLowerCase();
    return words.every(word => text.includes(word));
  });
  return <Modal id="app-settings-dialog" title={t("Settings")} onClose={onClose} className="w-[840px] max-h-[90dvh]" bodyClassName="p-0">
    <div className="settings-content">
      <div className="settings-search">
        <Search className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <input ref={searchRef} id="settings-search" type="search" value={query} placeholder={t("Search settings…")} aria-label={t("Search settings")} onChange={event => setQuery(event.target.value)} />
        {query && <Button variant="ghost" size="icon" type="button" aria-label={t("Clear search")} onClick={clearSearch}><X data-icon="inline-start" /></Button>}
      </div>
      <div className="settings-layout">
        <nav className="settings-navigation" aria-label={t("Settings categories")}>
          {categories.map(({ id, label, icon: Icon }) => <Button key={id} data-category={id} type="button" variant={!words.length && category === id ? "secondary" : "ghost"}
            aria-current={!words.length && category === id ? "page" : undefined} className="justify-start" onClick={() => { setCategory(id); setQuery(""); }}>
            <Icon data-icon="inline-start" />{t(label)}
          </Button>)}
        </nav>
        <main className="settings-results" aria-label={t("Settings results")}>
          <p role="status" className="mb-2 text-xs text-muted-foreground">{t("{count} settings", { count: visible.length })}</p>
          {categories.filter(item => item.id !== "all").map(group => {
            const settings = visible.filter(item => item.category === group.id);
            return settings.length ? <section key={group.id} aria-labelledby={`settings-category-${group.id}`}>
              <div className="flex items-center justify-between gap-3" data-mcp-status={group.id === "mcp" ? true : undefined}>
                <h3 id={`settings-category-${group.id}`} className="settings-section-title">{t(group.id === "mcp" ? "MCP status" : group.label)}</h3>
                {group.id === "mcp" && <Badge variant={mcp.status?.enabled ? "default" : "secondary"} role="status" aria-live="polite">
                  {t(mcp.status ? mcp.status.enabled ? "Started" : "Not started" : mcp.error ? "Unavailable" : "Loading…")}
                </Badge>}
              </div>
              {group.id === "mcp" && (mcp.error || mcp.status?.configurationError) && <Alert id="mcp-settings-error" variant="destructive" className="mb-3">
                <AlertDescription>{t(mcp.error || mcp.status?.configurationError || "")}
                  {!mcp.status && <Button type="button" variant="outline" size="sm" disabled={mcp.busy} onClick={() => void mcp.refresh()}>{t("Retry")}</Button>}
                </AlertDescription>
              </Alert>}
              {settings.map(setting => <div key={setting.id} className="settings-row" data-setting={setting.id}>
                <div className="min-w-0">{setting.id === "theme" || setting.id === "mcp-connection" ? <p id={`${setting.id}-label`} className="text-sm font-medium">{t(setting.title)}</p> : <label className="text-sm font-medium" htmlFor={setting.id === "language" ? "locale-select" : setting.id}>{t(setting.title)}</label>}
                  {setting.description && <p id={`${setting.id}-description`} className="mt-1 text-xs leading-relaxed text-muted-foreground">{t(setting.description)}</p>}
                </div>
                <div className={cn("settings-control", setting.id === "cursor-blink" && "settings-control-switch")}>{setting.control}</div>
              </div>)}
            </section> : null;
          })}
          {!visible.length && <Empty><EmptyHeader><EmptyMedia variant="icon"><Search /></EmptyMedia><EmptyTitle>{t("No settings found")}</EmptyTitle><EmptyDescription>{t("Try another keyword, such as language, font, or cursor.")}</EmptyDescription></EmptyHeader><EmptyContent><Button type="button" variant="outline" onClick={clearSearch}>{t("Clear search")}</Button></EmptyContent></Empty>}
          {visible.some(item => item.category === "terminal") && <div className="settings-preview" aria-label={t("Font preview")}><span>coterm &gt; </span>{t("The quick brown fox jumps over the lazy dog.")}<br />0123456789 · 中文终端预览</div>}
        </main>
      </div>
      <footer className="settings-footer">
        <p className="text-xs text-muted-foreground">{t("Display changes apply immediately. MCP port changes apply after saving.")}</p>
        <Button type="button" variant="ghost" size="sm" onClick={() => setPreferences({ ...DEFAULT_PREFERENCES })}><RotateCcw data-icon="inline-start" />{t("Reset font and cursor settings")}</Button>
      </footer>
    </div>
  </Modal>;
}
