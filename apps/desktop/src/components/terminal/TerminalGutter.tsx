import { CheckSquare, Square, Bot, User, AlertCircle, CheckCircle2 } from "lucide-react";
import type { CommandBlock } from "./types";

interface TerminalGutterProps {
  blocks: CommandBlock[];
  onToggleSelect: (id: string) => void;
  onAskAgentFix?: (block: CommandBlock) => void;
}

export function TerminalGutter({
  blocks,
  onToggleSelect,
  onAskAgentFix,
}: TerminalGutterProps) {
  if (blocks.length === 0) return null;

  return (
    <div className="flex w-52 flex-col border-r border-border bg-card text-xs">
      <div className="flex h-8 items-center justify-between border-b border-border px-2.5 text-[11px] font-semibold text-muted-foreground">
        <span>命令块 (Blocks)</span>
        <span className="text-[10px] text-muted-foreground">
          已选 {blocks.filter((b) => b.isSelected).length}
        </span>
      </div>

      <div className="flex-1 overflow-y-auto p-1.5 flex flex-col gap-1.5">
        {blocks.map((block) => {
          const isFailed = block.exitCode !== null && block.exitCode !== 0;

          return (
            <div
              key={block.id}
              className={`group relative flex flex-col rounded border p-1.5 transition ${
                block.isSelected
                  ? "border-primary/30 bg-accent"
                  : isFailed
                  ? "border-rose-500/30 bg-coterm-danger-surface hover:border-rose-500/50"
                  : "border-border bg-card hover:border-border"
              }`}
            >
              {/* 头部：复选框、序号、身份、状态 */}
              <div className="flex items-center justify-between text-[11px]">
                <div className="flex items-center gap-1.5">
                  <button
                    onClick={() => onToggleSelect(block.id)}
                    className="text-muted-foreground hover:text-primary"
                    title="勾选此块供 Agent 读取上下文"
                  >
                    {block.isSelected ? (
                      <CheckSquare className="h-3.5 w-3.5 text-primary" />
                    ) : (
                      <Square className="h-3.5 w-3.5" />
                    )}
                  </button>
                  <span className="font-mono text-muted-foreground">#{block.index}</span>
                </div>

                <div className="flex items-center gap-1">
                  {block.actor === "agent" ? (
                    <span className="flex items-center gap-0.5 rounded bg-accent px-1 py-0.2 text-[10px] text-primary">
                      <Bot className="h-2.5 w-2.5" />
                      <span>Agent</span>
                    </span>
                  ) : (
                    <span className="flex items-center gap-0.5 rounded bg-card px-1 py-0.2 text-[10px] text-muted-foreground">
                      <User className="h-2.5 w-2.5" />
                      <span>You</span>
                    </span>
                  )}

                  {block.exitCode !== null && (
                    isFailed ? (
                      <span title={`失败 (退出码: ${block.exitCode})`}>
                        <AlertCircle className="h-3 w-3 text-destructive" />
                      </span>
                    ) : (
                      <span title="成功">
                        <CheckCircle2 className="h-3 w-3 text-success" />
                      </span>
                    )
                  )}
                </div>
              </div>

              {/* 命令内容预览 */}
              <div className="mt-1 font-mono text-[11px] text-foreground truncate" title={block.command}>
                $ {block.command}
              </div>

              {/* 失败项快速修复药丸按钮 */}
              {isFailed && (
                <div className="mt-1.5 flex justify-end">
                  <button
                    onClick={() => onAskAgentFix?.(block)}
                    className="flex items-center gap-1 rounded bg-accent px-1.5 py-0.5 text-[10px] font-medium text-primary transition hover:bg-accent hover:text-primary-foreground"
                  >
                    <Bot className="h-2.5 w-2.5" />
                    <span>让 Agent 修复</span>
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
