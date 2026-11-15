# 前端工作台与视觉规范

整体依赖见[代码架构](architecture.md)，元数据字段与容量见[内部契约](contracts.md)，行为要求见[桌面交互](../requirements/desktop.md)。

## 组织与边界

桌面使用 React 18、TypeScript、Tailwind 与仓库内的 shadcn 组件。五种内置驱动的界面统一由宿主构建；不使用 Shadow DOM、独立驱动前端包或 Blob 源码加载。

| 入口 | 职责 |
|---|---|
| [manifests](../../crates/coterm-driver/manifests) 的 ui | 声明工作台类型、配置字段、默认值与辅助说明 |
| [definitions.ts](../../apps/desktop/src/workbenches/definitions.ts) | 静态读取五种内置元数据；未知驱动返回无描述 |
| [DriverConfiguration](../../apps/desktop/src/components/drivers/DriverConfiguration.tsx) | 名称校验、串口发现、提交状态和宿主表单生命周期 |
| [ConfigurationFields](../../apps/desktop/src/workbenches/ConfigurationFields.tsx) | 使用公共字段控件渲染受支持的 schema 子集，转换表单草稿 |
| [DriverWorkspace](../../apps/desktop/src/components/drivers/DriverWorkspace.tsx) | 按实例挂载视图与控制器，卸载时释放前端资源 |
| [WorkbenchViews](../../apps/desktop/src/workbenches/WorkbenchViews.tsx) | React 终端与流工作台布局、设置、搜索及发送控件 |
| [workbenches/common](../../apps/desktop/src/workbenches/common) | xterm、搜索、快捷键、命令块、通知、发送历史和流工作台复用 |
| [apps/shared](../../apps/shared) | 内部前端类型契约；不导出独立 UI 库 |

SSH 与 Local 复用终端布局，保留各自的会话和命令语义；SSH 另有后台任务列表和结果面板。Serial、TCP 与 Telnet 复用流工作台，其中 Telnet 只开放终端视图，Serial／TCP 可切换终端与字节调试。DTR／RTS 只属于串口；网络连接、代理和回显参数由网络控制器管理。

React 管理控件与列表结构；控制器保留已有的状态补取、回放、连接身份、输入队列和协议适配。xterm 使用独立叶容器，命令块叠层以 marker 定位，由 React 渲染块号和横向边界。异步控制器创建的列表使用局部 React 根，卸载幂等并拒绝迟到更新；容器快速重挂载时通过所有权标识阻止旧清理任务卸载新视图，不把协议控制改成通用状态框架。

## 配置与生命周期

宿主静态导入的配置元数据仅用于展示及基础校验；后端声明仍是最终业务参数和状态校验来源，不把展示 schema 当作授权边界，也不宣称支持完整 JSON Schema。新增实例与恢复不自动连接设备。

串口配置先启动驱动再查询端口；初始加载、鼠标打开与键盘打开可刷新，单页并发查询合并。离开配置页后不继续发现或修改页面。端口选项保留连接值，仅在展示时去除重复端口后缀。枚举失败与无端口分别提示。

标签隐藏保留实例视图，关闭标签清理前端监听、计时器和显示对象，不销毁后端实例。重新打开按后端状态恢复。主题、语言与字号变化原地更新，不重建终端或连接。所有运行操作仍通过绑定目标和来源的 Host API。

## 圆角与主题

主题变量统一定义在 [index.css](../../apps/desktop/src/index.css) 的根节点与深色模式中；Tailwind 和领域 CSS 使用同一组变量。

| 变量／类 | 计算 | 用途 |
|---|---|---|
| --radius／rounded-lg | 0.625rem 基准 | 独立卡片、弹窗、通知、复合切换器容器 |
| --radius-md／rounded-md | 基准减 2px | 按钮、选项、徽标、块号与关闭操作 |
| --radius-sm／rounded-sm | 基准减 4px | 输入、下拉、多行文本、快捷键标记 |
| rounded-full／50% | 正圆 | 状态灯、头像；不用于普通按钮或切换器 |

贴边工具栏和托盘保持布局连续，内部操作项仍使用标准圆角。终端命令块只画横线，不增加左右边框。不得在业务组件中写固定像素圆角、单向深色回退或私有调色板。

界面使用 background、foreground、card、muted、primary、destructive、warning 等语义色。工作台的 --coterm-* 变量引用同一套主题；xterm 使用解析后的终端色板。颜色常量集中在主题定义，领域组件不自行补黑色或白色默认值。阴影和遮罩使用 --shadow-color、--overlay、--coterm-shadow-panel、--coterm-shadow-up，明暗主题分别定义。

界面字体来自 --coterm-font-sans，终端和输出来自 --coterm-font-mono；主题与字号监听在卸载时注销。组件不自行写入固定系统字体栈。

## 公共组件

- 按钮复用 Button，以 variant 和 size 表达差异；提供一致的禁用、悬停与键盘焦点状态。只有尺寸、布局和领域状态可局部调整。
- 表单使用 Field、FieldLabel、Input、Select、Textarea；保留原生校验与选择器键盘行为。输入高度默认 h-9，搜索等紧凑场景可显式缩小。
- 设置开关使用 Switch；互斥设置使用现有 ToggleGroup。复合容器与内部选项分别使用标准圆角。
- 业务弹窗与秘密输入都复用 Modal。Modal 基于原生 dialog，提供焦点约束、Escape、关闭和统一遮罩，不依赖新的 Dialog 库。
- 秘密输入在投递前及卸载时清空；取消、超时和宿主条目消失后，迟到提交不再投递。密码不写入配置或普通提示。
- 辅助说明使用 title 与 aria-description，随语言切换；真实错误和连接状态保持可见。
- 通知和任务列表由 React 更新，保留有界数量、文本安全、焦点及滚动位置；普通通知不抢焦点，悬停或键盘阅读时暂停消失。

## 构建与验收

pnpm build 构建唯一产品前端，工作台按需加载。pnpm test:build 生成浏览器夹具，复用同一宿主组件与控制器，产物不进入产品调用链。构建顺序和命令见[开发与验收](development.md)。

回归应覆盖两种主题、动态圆角、宽窄窗口、配置校验、端口刷新、输入和搜索、命令块选择、任务列表、认证取消与卸载清理。浏览器结果不能替代原生 WebView 或其他操作系统的验收；有效覆盖见[验收状态](verification.md)。
