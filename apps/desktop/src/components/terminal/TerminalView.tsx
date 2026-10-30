import { useState } from "react";
import { useTerminal } from "./useTerminal";
import { TerminalGutter } from "./TerminalGutter";
import type { CommandBlock, GhostTextProposal } from "./types";
import { Check, X, ShieldAlert, Sparkles, Sidebar } from "lucide-react";

interface TerminalViewProps {
  onData?: (data: string) => void;
  blocks?: CommandBlock[];
  ghostProposal?: GhostTextProposal | null;
  onApproveGhost?: (command: string) => void;
  onRejectGhost?: () => void;
  onAskAgentFix?: (block: CommandBlock) => void;
}

export function TerminalView({
  onData,
  blocks = [],
  ghostProposal = null,
  onApproveGhost,
  onRejectGhost,
  onAskAgentFix,
}: TerminalViewProps) {
  const [showGutter, setShowGutter] = useState(true);
  const [localBlocks, setLocalBlocks] = useState<CommandBlock[]>(blocks);
  const [editedGhostCmd, setEditedGhostCmd] = useState<string>("");

  const { containerRef, clear, focus } = useTerminal({
    onData,
  });

  const handleToggleSelect = (id: string) => {
    setLocalBlocks((prev) =>
      prev.map((b) => (b.id === id ? { ...b, isSelected: !b.isSelected } : b))
    );
  };

  return (
    <div className="relative flex h-full w-full overflow-hidden rounded border border-border bg-card">
      {/* 左侧可折叠的命令块 Gutter */}
      {showGutter && (
        <TerminalGutter
          blocks={localBlocks}
          onToggleSelect={handleToggleSelect}
          onAskAgentFix={onAskAgentFix}
        />
      )}

      {/* 终端主体区 */}
      <div className="relative flex flex-1 flex-col overflow-hidden">
        {/* 终端工具栏 */}
        <div className="flex h-7 items-center justify-between border-b border-border bg-card px-3 text-[11px] text-muted-foreground">
          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowGutter(!showGutter)}
              className="flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-card hover:text-foreground"
              title="切换命令块侧栏"
            >
              <Sidebar className="h-3 w-3" />
              <span>块管理</span>
            </button>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={() => {
                clear();
                focus();
              }}
              className="rounded px-1.5 py-0.5 hover:bg-card hover:text-foreground"
            >
              清屏
            </button>
          </div>
        </div>

        {/* xterm.js 挂载容器 */}
        <div
          ref={containerRef}
          onClick={focus}
          className="flex-1 w-full p-2 overflow-hidden"
          style={{ height: "calc(100% - 28px)" }}
        />

        {/* 拟真行内 Ghost Text 审批悬浮条 (Decision 7) */}
        {ghostProposal && (
          <div className="absolute bottom-3 left-3 right-3 rounded-lg border border-primary/30 bg-card p-2.5 shadow-2xl backdrop-blur">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2 text-xs">
                <span className="flex items-center gap-1 text-primary font-semibold">
                  <Sparkles className="h-3.5 w-3.5" />
                  <span>Agent 待发指令:</span>
                </span>
                {ghostProposal.isRisky && (
                  <span className="flex items-center gap-1 rounded bg-coterm-danger-surface px-1.5 py-0.5 text-[10px] text-destructive">
                    <ShieldAlert className="h-3 w-3" />
                    <span>高危警告: {ghostProposal.riskReason}</span>
                  </span>
                )}
              </div>

              <div className="flex items-center gap-1.5">
                <button
                  onClick={() =>
                    onApproveGhost?.(editedGhostCmd || ghostProposal.command)
                  }
                  className="flex items-center gap-1 rounded bg-primary px-2 py-0.5 text-xs font-medium text-primary-foreground transition hover:bg-primary"
                >
                  <Check className="h-3 w-3" />
                  <span>回车放行 (Enter)</span>
                </button>
                <button
                  onClick={onRejectGhost}
                  className="flex items-center gap-1 rounded bg-card px-2 py-0.5 text-xs text-foreground transition hover:bg-card"
                >
                  <X className="h-3 w-3" />
                  <span>拦截 (Esc)</span>
                </button>
              </div>
            </div>

            {/* 可原地键盘纠偏的输入行 */}
            <div className="mt-1.5 flex items-center rounded bg-background px-2 py-1 font-mono text-xs">
              <span className="text-muted-foreground mr-2">$</span>
              <input
                type="text"
                defaultValue={ghostProposal.command}
                onChange={(e) => setEditedGhostCmd(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    onApproveGhost?.(editedGhostCmd || ghostProposal.command);
                  } else if (e.key === "Escape") {
                    onRejectGhost?.();
                  }
                }}
                className="flex-1 bg-transparent text-primary outline-none placeholder-purple-400/50"
                placeholder="直接在此编辑修改该命令以就地纠偏..."
              />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
