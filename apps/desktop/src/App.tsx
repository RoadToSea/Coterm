import { useState } from "react";
import {
  Terminal as TerminalIcon,
  Server,
  Cpu,
  Layers,
  ShieldCheck,
  Bot,
  Plus,
  Sliders,
} from "lucide-react";

export default function App() {
  const [activeTab, setActiveTab] = useState("local-1");
  const [isBypass, setIsBypass] = useState(false);
  const [guardrailEnabled, setGuardrailEnabled] = useState(true);

  return (
    <div className="flex h-screen w-screen flex-col bg-slate-950 text-slate-200">
      {/* 顶部标签栏 */}
      <header className="flex h-10 items-center justify-between border-b border-slate-800 bg-slate-900 px-3">
        <div className="flex items-center space-x-1">
          <div className="flex items-center space-x-2 pr-4 font-semibold text-emerald-400">
            <Layers className="h-5 w-5" />
            <span>Coterm</span>
          </div>

          <div className="flex items-center space-x-1">
            <button
              onClick={() => setActiveTab("local-1")}
              className={`flex items-center space-x-2 rounded-t px-3 py-1.5 text-xs font-medium transition ${
                activeTab === "local-1"
                  ? "bg-slate-950 text-slate-100 border-t-2 border-emerald-500"
                  : "bg-slate-900 text-slate-400 hover:bg-slate-800"
              }`}
            >
              <TerminalIcon className="h-3.5 w-3.5 text-sky-400" />
              <span>Local PowerShell</span>
            </button>
            <button className="rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-200">
              <Plus className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* 模式与安全控制区 */}
        <div className="flex items-center space-x-3 text-xs">
          <button
            onClick={() => setIsBypass(!isBypass)}
            className={`flex items-center space-x-1 rounded px-2.5 py-1 font-medium transition ${
              isBypass
                ? "bg-amber-500/20 text-amber-300 border border-amber-500/30"
                : "bg-emerald-500/20 text-emerald-300 border border-emerald-500/30"
            }`}
          >
            <Bot className="h-3.5 w-3.5" />
            <span>{isBypass ? "Bypass 模式 (全自动)" : "人类抢占 / 审核模式"}</span>
          </button>

          <button
            onClick={() => setGuardrailEnabled(!guardrailEnabled)}
            className={`flex items-center space-x-1 rounded px-2 py-1 transition ${
              guardrailEnabled
                ? "text-emerald-400 hover:bg-slate-800"
                : "text-rose-400 hover:bg-slate-800"
            }`}
            title="高危命令熔断保护"
          >
            <ShieldCheck className="h-4 w-4" />
            <span>{guardrailEnabled ? "熔断开启" : "熔断关闭"}</span>
          </button>
        </div>
      </header>

      {/* 主工作区 */}
      <div className="flex flex-1 overflow-hidden">
        {/* 左侧资产树与插件栏 */}
        <aside className="w-56 border-r border-slate-800 bg-slate-900/50 p-3">
          <div className="mb-4">
            <h2 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              会话连接
            </h2>
            <ul className="space-y-1 text-xs">
              <li>
                <button className="flex w-full items-center space-x-2 rounded px-2 py-1.5 text-slate-300 hover:bg-slate-800">
                  <TerminalIcon className="h-4 w-4 text-sky-400" />
                  <span>本地终端</span>
                </button>
              </li>
              <li>
                <button className="flex w-full items-center space-x-2 rounded px-2 py-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-300">
                  <Server className="h-4 w-4 text-emerald-400" />
                  <span>SSH 远程连接</span>
                </button>
              </li>
              <li>
                <button className="flex w-full items-center space-x-2 rounded px-2 py-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-300">
                  <Cpu className="h-4 w-4 text-amber-400" />
                  <span>串口调试 (UART)</span>
                </button>
              </li>
            </ul>
          </div>

          <div>
            <h2 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              插件生态
            </h2>
            <button className="flex w-full items-center space-x-2 rounded px-2 py-1.5 text-xs text-slate-400 hover:bg-slate-800 hover:text-slate-300">
              <Sliders className="h-4 w-4 text-purple-400" />
              <span>插件市场 (Market)</span>
            </button>
          </div>
        </aside>

        {/* 右侧终端与命令块工作台 */}
        <main className="relative flex flex-1 flex-col bg-slate-950 p-3">
          <div className="flex flex-1 items-center justify-center rounded border border-slate-800/80 bg-black/60 p-4 font-mono text-xs text-slate-400">
            <div className="text-center">
              <p className="text-sm font-semibold text-slate-300">Coterm Terminal Ready</p>
              <p className="mt-1 text-slate-500">
                等待启动会话通道或挂载 xterm.js 仿真组件...
              </p>
            </div>
          </div>
        </main>
      </div>

      {/* 底部状态栏 */}
      <footer className="flex h-6 items-center justify-between border-t border-slate-800 bg-slate-900 px-3 text-[11px] text-slate-500">
        <div className="flex items-center space-x-4">
          <span className="flex items-center space-x-1">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-400"></span>
            <span>协议: Local PTY</span>
          </span>
          <span>MCP 网关: 监听中 (端口 8090)</span>
        </div>
        <div>
          <span>Coterm v0.1.0</span>
        </div>
      </footer>
    </div>
  );
}
