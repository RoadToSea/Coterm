# 命令消息块与选择

## 交互与边界

组件采用代码编辑器式布局：左侧是块号，右侧是一条命令及其回复；每个块有独立边框。点击块号切换该块的选择状态，允许多选；选中块以背景、边框及左侧色条高亮。点击或拖选正文只进行原生文本选择，不改变块选择，便于复制命令与回复。

块号按钮支持 Tab 聚焦、Enter / Space 切换；上 / 下方向键和 Home / End 移动块号焦点，不自动改变选择。回复更新复用相同 ID 的 DOM，未改变的文本不会被重建；不自动滚到列表底部。组件继承浅色 / 深色 `--coterm-*` 变量，可挂载到 Shadow DOM。

这是插件共用的展示与选择组件，不是宿主交互弹窗，也不是终端仿真器。组件不执行命令、不解析协议或 ANSI 控制序列、不猜测命令边界；插件负责将一条命令与对应回复归并成块，保证实例隔离和数据脱敏。

## 类型与参数

类型定义见 [plugin-api.ts](../src/plugin-api.ts)，实现见 [block-list.ts](../src/components/block-list.ts)。

| `CommandBlock` 字段 | 必填 | 含义 |
|---|:---:|---|
| `id: string` | 是 | 当前列表内唯一、跨更新稳定的块 ID；不能使用会随插入而变化的数组位置 |
| `index: number` | 是 | 左侧显示的块号，由插件决定起点与编号 |
| `command: string` | 是 | 原样显示的命令文本，支持多行 |
| `output?: string` | 否 | 该命令所属的明文回复；缺失或空字符串时隐藏回复区 |
| `actor: "human" \| "robot"` | 是 | 来源标签，分别显示 Human / Robot |
| `exitCode?: number \| null` | 否 | 非零时展示失败标记与退出码说明；组件不推断执行结果 |
| `selected?: boolean` | 否 | 指定时覆盖本地选择；更新中省略时按 ID 保留本地选择，新块默认未选 |

| `BlockListProps` 字段 | 含义 |
|---|---|
| `blocks: CommandBlock[]` | 当前完整、有界快照；更新时按 ID 增删或重排 |
| `onSelectionChange?(ids: string[])` | 用户操作后回调，返回当前显示顺序中的已选 ID；外部 update 不触发该回调 |
| `ariaLabel?: string` | 列表的可访问名称，默认 Command blocks，可由插件提供本地化文案 |
| `emptyText?: string` | 空状态文案，默认 No command blocks yet. |

`update(props)` 接收新的完整 props。插件若显式提供 `selected`，需在回调中同步自身选择数据，避免下一次快照用旧值覆盖用户选择；省略该字段则由组件保留选择。删除块会移除对应选择，插件持有的上下文也应按当前 blocks 筛选。

本次沿用现有 `components.blocks` 挂载签名，SDK 契约版本仍为 1.0；新增数据字段均可选，旧调用方无需修改。旧宿主渲染器不保证展示新增字段；需要固定 UI 实现版本的插件可直接导入 `@coterm/plugin-sdk/components/block-list` 中的 `mountBlockList` 随插件打包。

## 插件接入示例

下面代码放在插件 `mount(container, api)` 内；数据仅为展示样例，不会执行命令：

```ts
import type { CommandBlock } from "@coterm/plugin-sdk";

let blocks: CommandBlock[] = [
  { id: "pwd-1", index: 1, command: "pwd", output: "/home/operator", actor: "human", exitCode: 0 },
  { id: "platform-2", index: 2, command: "uname -sr", output: "Linux 6.6.36-riscv64", actor: "robot" },
];
let selectedIds: string[] = [];
const view = api.components.blocks(container, {
  blocks,
  ariaLabel: "命令与回复",
  emptyText: "暂无命令消息",
  onSelectionChange(ids) {
    selectedIds = ids;
    // 上下文只由插件使用；组件不会自动提交给 Agent 或访问后端。
  },
});

// 插件收到已确认归属的增量回复后，更新对应块。
blocks = blocks.map(block => block.id === "platform-2"
  ? { ...block, output: `${block.output}\n设备输出的下一行` }
  : block);
view.update({
  blocks,
  ariaLabel: "命令与回复",
  emptyText: "暂无命令消息",
  onSelectionChange: ids => { selectedIds = ids; },
});
const selectedBlocks = blocks.filter(block => selectedIds.includes(block.id));
// 插件自行处理 selectedBlocks；卸载时调用 view.unmount()。
```

插件应限制块数与每块输出大小，分页或裁剪较早数据，并明确提示数据缺口；组件只渲染当前快照，不为每次 update 额外积累历史。回复作为纯文本写入 DOM，不解释 HTML。选择刷新不重新创建当前消息节点；实际回复内容变化时浏览器原有文本选区可能自然改变，不承诺保持被改写文本的选区。

返回的 view 提供 `update(props)` 和 `unmount()`。卸载会移除 DOM 及事件监听；重复卸载安全，卸载后的更新与旧控件点击无效。组件不连接或断开后端实例。

## 预览与验证

仓库根目录执行以下命令构建同一份真实 SDK 源码并启动本地静态服务：

```bash
pnpm test:build
python -m http.server 4174 --bind 127.0.0.1
```

浏览器访问：

```text
http://127.0.0.1:4174/packages/plugin-sdk/examples/command-blocks.html
```

[示例页面](../examples/command-blocks.html) 可切换主题、选择块并追加示例回复；没有设备连接和命令执行。浏览器验收使用实际组件、Shadow DOM、原生键盘事件及计算样式，不用 UI 替身：

```bash
pnpm build
pnpm test:build
node --import ./scripts/test-ts-loader.mjs --test tests/command-blocks-browser.test.mjs tests/visual-tokens-layout.test.mjs
```

需要 Chrome / Edge / Chromium。无浏览器时测试明确 skip，不能当作通过。测试覆盖多选与高亮、正文复制、回复更新、选择覆盖与清理、键盘、空状态、纯文本注入防护、滚动位置、主题继承、320px 窄窗口以及卸载。
