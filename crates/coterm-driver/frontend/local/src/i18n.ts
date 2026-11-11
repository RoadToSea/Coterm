import type { DriverI18n } from "../../../../../apps/shared/index";

const zhCN: Readonly<Record<string, string>> = {
  "Find in terminal": "在终端中查找",
  "No matches": "未找到匹配项",
  "{index} of {total}": "第 {index} / {total} 项",
  "{total} matches": "{total} 个匹配项",
  "Previous match (Shift+Enter)": "上一处（Shift+Enter）",
  "Next match (Enter)": "下一处（Enter）",
  "Close search (Esc)": "关闭搜索（Esc）",
  "Could not copy terminal selection. Try copying again.": "无法复制终端选区，请重试复制。",
  "Open local terminal": "创建本地终端",
  "Shell": "Shell",
  "Leave empty to detect the system shell.": "留空则自动探测系统 Shell。",
  "Common shells": "常用 Shell",
  "Working directory": "工作目录",
  "Leave empty to start in your home directory.": "留空则在用户主目录启动。",
  "Open terminal": "打开终端",
  "Local": "本地",
  "running": "运行中",
  "exited": "已退出",
  "exit {code}": "退出码 {code}",
  "Restart shell": "重启 Shell",
  "The shell exited. Restart it to keep using this instance.": "Shell 已退出，重启后可继续使用该实例。",
  "Driver connection closed.": "驱动连接已关闭。",
  "Replay limit reached; waiting for the next output event.": "已达到回放上限，等待下一条输出事件。",
  "Input queue is full. Wait before typing or pasting more.": "输入队列已满，请稍后再输入或粘贴。",
  "Unknown shell family; this instance can run a terminal but not Agent commands.": "无法识别的 Shell 家族：可以开终端，但不能执行 Agent 命令。",
};

export function createTranslator(i18n?: DriverI18n) {
  return (message: string, values: Record<string, string | number> = {}): string => {
    const template = i18n?.locale === "zh-CN" ? zhCN[message] ?? message : message;
    return template.replace(/\{(\w+)\}/g, (match, key: string) => String(values[key] ?? match));
  };
}
