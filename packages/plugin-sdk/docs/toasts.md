# 非模态提示

插件前端直接导入公共组件，随自己的 bundle 构建；不需要后端生成 UI，不增加宿主权限或跨进程消息。

```ts
import { mountToasts } from "@coterm/plugin-sdk/components/toasts";

// 容器提供定位上下文；支持普通 DOM 和 Shadow DOM。
container.style.position = "relative";
const notices = mountToasts(container, {
  durationMs: 4000,
  maxVisible: 3,
  ariaLabel: "提示",
  closeLabel: "关闭提示",
});

notices.show({ message: "请先断开连接再修改波特率。", tone: "warning" });
const dismiss = notices.show({ message: "连接失败，请检查设备。", tone: "error" });
// 按需提前关闭这一条：dismiss();
// 语言切换：notices.updateLabels({ ariaLabel: "Notifications", closeLabel: "Dismiss" });
// 插件卸载时：notices.unmount();
```

## 接口

| 接口 / 字段 | 说明 |
|---|---|
| `mountToasts(container, options?)` | 创建右上角提示栈，返回独立控制器；父容器应提供定位上下文 |
| `show({ message, tone?, durationMs? })` | 显示纯文本，返回该条提示的关闭函数；空白消息忽略 |
| `tone` | `error` 红色、`warning` 黄色、`info` 主题强调色；默认 `info` |
| `durationMs` | 默认 4000 ms，有效范围限制为 500–30000 ms；单条可覆盖全局设置 |
| `maxVisible` | 默认 3，限制为 1–5；超出时淘汰最早的提示 |
| `updateLabels({ ariaLabel?, closeLabel? })` | 更新提示区域与所有关闭按钮的可访问名称 |
| `clear()` | 移除当前提示、取消对应计时器；控制器仍可使用 |
| `unmount()` | 移除节点、样式和计时器；后续 `show()` 不再产生 UI |

相同级别及文本合并为一条并重新计时；单条最多 2048 个 UTF-16 码元。文案使用 `textContent`，不解释 HTML。调用方负责提供面向用户的文案，不传入密码、Token、连接凭证或未经脱敏的协议内容。

## 交互与样式

- 圆角矩形、主题底色与语义色边框；继承 `--coterm-*` 变量，适配浅色、深色和窄容器。错误使用 `--coterm-danger`，警告使用 `--coterm-warning` 或宿主 `--warning`，均有 fallback。
- 非模态，不弹出 `dialog`、不覆盖背景、不锁定编辑器，也不自动移动焦点。区域空白部分不拦截指针，提示本身支持关闭。
- 默认自动消失；悬停或键盘聚焦提示时暂停剩余计时，移出后继续，便于阅读。相同消息重发会刷新时长。
- 错误 / 警告使用 `role="alert"`，普通提示使用 `role="status"`；关闭按钮有可访问名称。动画只改变 opacity / transform，遵循减少动画设置。

串口插件已接入：运行中修改不可变参数、输入校验、队列警告和操作失败均通过此组件提示。其他插件可直接引用同一模块；审批、认证和秘密输入继续使用宿主的正式交互 API。

## 验证

```bash
pnpm test:build
node --import ./scripts/test-ts-loader.mjs --test tests/toasts-browser.test.mjs
```

用例在真实浏览器及 Shadow DOM 中检查颜色、圆角、不抢焦点、自动消失、悬停 / 聚焦暂停、去重、容量、关闭、纯文本、本地化和卸载清理。
