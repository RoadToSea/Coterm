import type { PluginI18n } from "@coterm/plugin-sdk";

const zhCN: Readonly<Record<string, string>> = {
  "Could not copy terminal selection. Try copying again.": "无法复制终端选区，请重试复制。",
  "Host": "主机",
  "Port": "端口",
  "Username": "用户名",
  "Open SSH": "创建 SSH 实例",
  "Loading…": "加载中…",
  "Connect": "连接",
  "Disconnect": "断开连接",
  "Connected": "已连接",
  "Disconnected": "未连接",
  "Background jobs": "后台任务",
  "Stop": "停止",
  "Close view": "关闭视图",
  "JOBS": "任务",
  "Remove job from list": "从列表移除任务",
  "cancelled": "已取消",
  "timed out": "已超时",
  "running": "运行中",
  "exit {code}": "退出码 {code}",
  "Plugin connection closed.": "插件连接已关闭。",
  "Verify SSH server": "验证 SSH 服务器",
  "Compare this fingerprint with the server administrator before continuing.": "请与服务器管理员核对指纹后再继续。",
  "SSH authentication": "SSH 认证",
  "Password for {server}": "{server} 的密码",
  "Replay limit reached; waiting for the next output event.": "已达到回放上限，等待下一条输出事件。",
  "Input queue is full. Wait before typing or pasting more.": "输入队列已满，请稍后再输入或粘贴。",
};

export function createTranslator(i18n?: PluginI18n) {
  return (message: string, values: Record<string, string | number> = {}): string => {
    const template = i18n?.locale === "zh-CN" ? zhCN[message] ?? message : message;
    return template.replace(/\{(\w+)\}/g, (match, key: string) => String(values[key] ?? match));
  };
}
