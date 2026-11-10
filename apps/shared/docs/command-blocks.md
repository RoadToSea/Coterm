# 命令消息块与选择

## 接入与职责

驱动前端直接引用共享前端组件，构建时将组件打入驱动的单文件前端 bundle：

```ts
import { mountBlockList } from "../../../../apps/shared/components/block-list";
import type { CommandBlock } from "../../../../apps/shared/index";
```

数据流为：**驱动后端生成块 → 基座转交操作结果 / 实例事件 → 驱动前端接收并适配 → `mountBlockList` / `view.update`**。

- 驱动后端负责命令与回复的归属、稳定块 ID、编号、来源及执行结果，并限制保留量、处理截断和秘密脱敏。
- 驱动前端通过实例绑定的 `api.invoke` / `api.subscribeEvents` 获取后端数据，必要时把驱动自己的字段适配为下表的 `CommandBlock`，再交给组件。
- 组件负责边框、块号、正文显示与选择交互；`selected` 及已选 ID 是前端状态，不要求后端持久化或随回复回传。
- 基座负责实例路由、审批和已有通信通道；消息块不增加宿主领域操作，也不要求宿主解析终端输出。

下表是组件接受的前端数据形状，不是新增的后端通用协议。驱动后端返回结构化块数据，驱动前端通过基座接收并适配后交给组件；操作名、事件名与后端字段由具体驱动约定。组件只显示纯文本。

## 交互与边界

组件采用代码编辑器式布局：左侧是块号，右侧是一条命令及其回复；每个块有独立边框。点击块号切换该块的选择状态，允许多选；选中块以背景、边框及左侧色条高亮。点击或拖选正文只进行原生文本选择，不改变块选择，便于复制命令与回复。

块号按钮支持 Tab 聚焦、Enter / Space 切换；上 / 下方向键和 Home / End 移动块号焦点，不自动改变选择。回复更新复用相同 ID 的 DOM，未改变的文本不会被重建；不自动滚到列表底部。组件继承浅色 / 深色 `--coterm-*` 变量，可挂载到 Shadow DOM。

这是驱动共用的展示与选择组件，不是宿主交互弹窗，也不是终端仿真器。组件不执行命令、不解析协议或 ANSI 控制序列、不猜测命令边界；驱动负责将一条命令与对应回复归并成块，保证实例隔离和数据脱敏。

## 类型与参数

类型定义见 [driver-api.ts](../driver-api.ts)，实现见 [block-list.ts](../components/block-list.ts)。

| `CommandBlock` 字段 | 必填 | 含义 |
|---|:---:|---|
| `id: string` | 是 | 当前列表内唯一、跨更新稳定的块 ID；不能使用会随插入而变化的数组位置 |
| `index: number` | 是 | 左侧显示的块号，由驱动决定起点与编号 |
| `command: string` | 是 | 原样显示的命令文本，支持多行 |
| `output?: string` | 否 | 该命令所属的明文回复；缺失或空字符串时隐藏回复区 |
| `statusText?: string` | 否 | 驱动本地化后的状态 / 通道标签，显示在块头部，与回复正文分开 |
| `actor: "human" \| "robot"` | 是 | 来源标签，分别显示 Human / Robot |
| `exitCode?: number \| null` | 否 | 非零时展示失败标记与退出码说明；组件不推断执行结果 |
| `selected?: boolean` | 否 | 指定时覆盖本地选择；更新中省略时按 ID 保留本地选择，新块默认未选 |

| `BlockListProps` 字段 | 含义 |
|---|---|
| `blocks: CommandBlock[]` | 当前完整、有界快照；更新时按 ID 增删或重排 |
| `onSelectionChange?(ids: string[])` | 用户操作后回调，返回当前显示顺序中的已选 ID；外部 update 不触发该回调 |
| `ariaLabel?: string` | 列表的可访问名称，默认 Command blocks，可由驱动提供本地化文案 |
| `emptyText?: string` | 空状态文案，默认 No command blocks yet. |

`update(props)` 接收新的完整 props。驱动若显式提供 `selected`，需在回调中同步自身选择数据，避免下一次快照用旧值覆盖用户选择；省略该字段则由组件保留选择。删除块会移除对应选择，驱动持有的上下文也应按当前 blocks 筛选。

新接入使用直接导入的 `mountBlockList`，组件实现版本随驱动构建固定。已有 `api.components.blocks` 保留兼容，它由宿主提供实现；旧宿主渲染器不保证展示新增字段。前端契约版本为 2.0，新增数据字段均可选。

## 驱动接入示例

下面展示驱动前端的挂载与更新逻辑；导入语句置于模块顶部，其余代码放在驱动 `mount(container, api)` 内。`receiveBlocks` 接收驱动前端从后端结果 / 事件适配得到的完整快照：

```ts
import { mountBlockList } from "../../../../apps/shared/components/block-list";
import type { BlockListProps, CommandBlock } from "../../../../apps/shared/index";

// 此类型只描述适配后的显示数据；后端具体操作和事件契约属于驱动。
type DisplayBlock = Omit<CommandBlock, "selected">;
let blocks: DisplayBlock[] = [];
let selectedIds: string[] = [];
const props = (): BlockListProps => ({
  blocks,
  ariaLabel: "命令与回复",
  emptyText: "暂无命令消息",
  onSelectionChange(ids) {
    selectedIds = ids;
    // 上下文只由驱动使用；组件不会自动提交给 Agent 或访问后端。
  },
});
const view = mountBlockList(container, props());

function receiveBlocks(snapshot: DisplayBlock[]) {
  blocks = snapshot;
  selectedIds = selectedIds.filter(id => blocks.some(block => block.id === id));
  view.update(props());
}
function selectedBlocks() {
  return blocks.filter(block => selectedIds.includes(block.id));
}
// 将 receiveBlocks 接到驱动自己的操作结果 / 实例事件处理器。
// 驱动自行使用 selectedBlocks()；卸载时注销订阅并调用 view.unmount()。
```

驱动应限制块数与每块输出大小，分页或裁剪较早数据，并明确提示数据缺口；组件只渲染当前快照，不为每次 update 额外积累历史。回复作为纯文本写入 DOM，不解释 HTML。选择刷新不重新创建当前消息节点；实际回复内容变化时浏览器原有文本选区可能自然改变，不承诺保持被改写文本的选区。

返回的 view 提供 `update(props)` 和 `unmount()`。卸载会移除 DOM 及事件监听；重复卸载安全，卸载后的更新与旧控件点击无效。组件不连接或断开后端实例。

### SSH 实际接入

[SSH 前端适配](../../../drivers/driver-ssh/frontend/src/blocks.ts) 直接导入该组件，通过实例绑定的 `ssh.blocks` 读取后端有界分页快照，由 `ssh.blocks.changed` 通知失效。驱动后端记录真实获批的 PTY / exec 操作及其回复，前端把后端字段转换为 `CommandBlock`，并用 `statusText` 展示通道和执行状态。完整数据契约和上限见 [SSH 消息块](../../../docs/architecture/ssh-driver.md#命令消息块)。

## 预览与验证

### 终端行号栏

`apps/shared/components/terminal-blocks` 导出 `mountTerminalBlocks(gutter, overlay, terminal, options)`，用于已有 xterm.js 终端，不替代终端输出或协议实现。驱动通过实例绑定的数据通知 `begin(block, lineOffset = 0)` / `end(id, lineOffset?)`；`lineOffset` 是相对当前光标行的 marker 偏移，可将已回显并软折行的人工命令锚定至首行，也可把显示结束位置放在下一条提示符前。省略 end 偏移时，默认当前行（光标在第 0 列时为前一行）；重复 end 保留首次确认的结束 marker。组件不识别提示符或判断执行边界，驱动须区分可靠执行边界与前端观察。组件使用 xterm marker 定位块号，使用单层 DOM 绘制上下横向分割线，不绘制左右竖线，行号栏也不绘制竖向边线；分割线按 border-box 定位，不重复叠加 decoration；相邻块自动以后一块首行作为前一块显示上限。调用 `update(blocks)` 后仅允许当前快照中的稳定 ID 被选择。`toggle(id)` 可连接未内嵌在 PTY 网格中的任务胶囊。`onSelectionChange(ids)` 供驱动把选择交给自己的后端上下文接口。按钮与高亮随可见 buffer 行呈现，响应滚动、渲染、尺寸和 buffer 清理；备用屏幕隐藏 PTY 标记。卸载注销终端监听、ResizeObserver 和键盘事件。

行号栏宽度按当前有效 PTY 块号最大字符数伸缩，最小 28px；仅显示数字，不加 `#` 前缀。使用行号栏自身的等宽字体计算，淘汰、更新及重置时同步收缩。顶部的红色圆形叉号用于取消全选（16px），`cancelLabel` / `refreshLocale(label)` 更新 tooltip 与可访问名称；空选择时禁用并保留红色，按钮具有键盘焦点样式。

选择区域持有 Esc 清除选择；主终端的 Esc 不被 共享前端模块拦截。驱动负责限制块和选择数量，后端再次按实例 / epoch 校验，且只允许人工来源修改选择。不要把原始终端输入、命令秘密、输出或用户可控终端文本编码为可信执行边界。SSH 人工 Enter 使用单独的 `boundary=observed` 观察语义，不能据此推断执行结果；命令和回复仍只通过人工明确选择后交给 Agent。

SSH 实际边界与选择接口见 [SSH 消息块](../../../docs/architecture/ssh-driver.md#命令消息块)。浏览器集成入口：`tests/ssh-blocks-browser.test.mjs`。

仓库根目录执行以下命令构建同一份真实共享前端源码并启动本地静态服务：

```bash
pnpm test:build
python -m http.server 4174 --bind 127.0.0.1
```

浏览器访问：

```text
http://127.0.0.1:4174/apps/shared/examples/command-blocks.html
```

[示例页面](../examples/command-blocks.html) 可切换主题、选择块并追加示例回复；没有设备连接和命令执行。浏览器验收使用实际组件、Shadow DOM、原生键盘事件及计算样式，不用 UI 替身：

```bash
pnpm build
pnpm test:build
node --import ./scripts/test-ts-loader.mjs --test tests/command-blocks-browser.test.mjs tests/visual-tokens-layout.test.mjs
```

需要 Chrome / Edge / Chromium。无浏览器时测试明确 skip，不能当作通过。测试覆盖多选与高亮、正文复制、回复更新、选择覆盖与清理、键盘、空状态、纯文本注入防护、滚动位置、主题继承、320px 窄窗口以及卸载。
