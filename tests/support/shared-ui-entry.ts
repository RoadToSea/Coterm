import css from "../../apps/desktop/src/index.css?inline";
const style = document.createElement("style"); style.textContent = css; document.head.append(style);
export { mountBlockList } from "../../apps/desktop/src/workbenches/common/block-list";
export { mountTerminalBlocks } from "../../apps/desktop/src/workbenches/common/terminal-blocks";
export { mountToasts } from "../../apps/desktop/src/workbenches/common/toasts";
