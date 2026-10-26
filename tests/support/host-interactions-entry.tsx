/**
 * 浏览器用例的挂载入口：只渲染真实的 HostInteractions 组件。
 * 页面侧从 App 里拆出来，是因为 App 同时挂 HostApprovals，两份订阅记录会互相干扰。
 */
import React from "react";
import ReactDOM from "react-dom/client";
import { HostInteractions } from "../../apps/desktop/src/components/host/HostInteractions";
import { createPluginAPI, retryHostAttention } from "../../apps/desktop/src/host-api";

window.__cotermHostInteractions = {
  createPluginAPI,
  retryHostAttention,
  mount() {
    const root = document.getElementById("root");
    if (!root) throw new Error("missing #root");
    this.root = ReactDOM.createRoot(root);
    this.root.render(React.createElement(React.StrictMode, null, React.createElement(HostInteractions, null)));
  },
  unmount() {
    this.root?.unmount();
  },
};
