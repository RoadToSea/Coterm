import type { PluginI18n } from "@coterm/plugin-sdk";

const zhCN: Readonly<Record<string, string>> = {
  // 配置界面
  "Port": "端口",
  "Baud rate": "波特率",
  "Data bits": "数据位",
  "Stop bits": "停止位",
  "Parity": "校验位",
  "None": "无",
  "Odd": "奇校验",
  "Even": "偶校验",
  "DTR high on open": "打开时置高 DTR",
  "RTS high on open": "打开时置高 RTS",
  "Port names come from the operating system, e.g. COM6 or /dev/ttyUSB0.": "端口名由操作系统提供，例如 COM6 或 /dev/ttyUSB0。",
  "Some boards reset when DTR changes; clear it if the device reboots unexpectedly.": "部分开发板会因 DTR 变化复位；设备意外重启时可取消勾选。",
  "Open serial port": "打开串口",
  "A serial port name is required.": "请填写串口名。",
  "DTR and RTS default to high; clear them for boards that reset on open.": "DTR 与 RTS 默认置高；会在打开时复位的板子请取消勾选。",

  // 通用
  "Serial": "串口",
  "Loading…": "加载中…",
  "Connected": "已连接",
  "Disconnected": "未连接",
  "Connect": "连接",
  "Disconnect": "断开连接",
  "Ports": "端口列表",
  "Terminal": "终端",
  "UART console": "UART 调试",
  "Plugin connection closed.": "插件连接已关闭。",
  "Replay limit reached; waiting for the next output event.": "已达到回放上限，等待下一条输出事件。",
  "Input queue is full. Wait before sending more.": "输入队列已满，请稍后再发送。",
  "No serial ports were reported by the operating system.": "操作系统未报告任何串口。",
  "Operation {status}": "操作{status}",
  "Show timestamps": "显示时间戳",
  "Timestamp": "时间戳",
  "ASCII": "ASCII",
  "HEX": "HEX",
  "Pause": "暂停",
  "Resume": "继续",
  "Clear": "清空",
  "Auto-scroll": "自动滚动",
  "Receive": "接收",
  "Send": "发送",
  "Input is HEX": "按 HEX 输入",
  "Line ending": "行结束符",
  "Text": "文本",
  "Send text, then Send (Enter sends, Shift+Enter adds a line)": "输入内容后发送（Enter 发送，Shift+Enter 换行）",
  "Repeat": "重复发送",
  "Repeat every {ms} ms": "每 {ms} 毫秒重复发送",
  "Start repeating": "开始重复",
  "Stop repeating": "停止重复",
  "RX {rx} B · TX {tx} B": "接收 {rx} B · 发送 {tx} B",
  "DTR": "DTR",
  "RTS": "RTS",
  "High": "高",
  "Low": "低",
  "HEX input must contain an even number of hexadecimal digits.": "HEX 输入必须是偶数个十六进制字符。",
  "Output gap: older data expired from the replay buffer": "输出缺口：较早的数据已从回放缓冲过期",
  "Repeat interval must be between 20 and 60000 ms.": "重复间隔必须在 20 到 60000 毫秒之间。",
  "Repeat stopped: the port is disconnected.": "重复发送已停止：端口已断开。",
  "Repeat stopped: the plugin connection closed.": "重复发送已停止：插件连接已关闭。",
  "Connected as {label}": "已连接 {label}",
  "The device sent {bytes} bytes while paused.": "暂停期间设备又收到 {bytes} 字节。",
};

export function createTranslator(i18n?: PluginI18n) {
  return (message: string, values: Record<string, string | number> = {}): string => {
    const template = i18n?.locale === "zh-CN" ? zhCN[message] ?? message : message;
    return template.replace(/\{(\w+)\}/g, (match, key: string) => String(values[key] ?? match));
  };
}

export type Translator = ReturnType<typeof createTranslator>;
