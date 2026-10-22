/**
 * 当前构建的真实浏览器验收：布局（1280/1024/768）、纯键盘折叠恢复、PluginCatalog 逐包诊断、
 * 真实 SSH 前端 Shadow DOM 与 SDK mountBlockList 的 token/fallback 计算样式验证、多审批卡片独立处理。
 *
 * 这里验证的是 App.tsx + index.css + plugin-sdk + coterm-plugin-ssh-ui 的实际浏览器渲染结果。
 * 没有浏览器时整组 skip 并写明原因；skip 不等于通过。
 */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  assertFreshBuild,
  delay,
  distDir,
  evaluate,
  findBrowser,
  openSession,
  rootDir,
  startStaticServer,
  waitForValue,
} from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

const browserPath = findBrowser();

/** 新建一页并注入宿主替身，返回可用页面对象。 */
async function openPage(session, scenario = defaultScenario, viewport = { width: 1280, height: 800 }) {
  const { client } = session;
  const target = await client.send("Target.createTarget", { url: "about:blank" });
  const attached = await client.send("Target.attachToTarget", { targetId: target.targetId, flatten: true });
  const sessionId = attached.sessionId;
  const send = (method, params = {}) => client.send(method, params, sessionId);
  const waitFor = (method, timeoutMs) => client.waitFor(method, timeoutMs, sessionId);

  await send("Page.enable");
  await send("Runtime.enable");
  await send("Log.enable");
  client.on("Runtime.consoleAPICalled", (params) => {
    console.log("[BROWSER CONSOLE]", ...params.args.map((a) => a.value ?? a.description));
  }, sessionId);
  client.on("Runtime.exceptionThrown", (params) => {
    console.error("[BROWSER EXCEPTION]", params.exceptionDetails.text, params.exceptionDetails.exception?.description);
  }, sessionId);
  await send("Page.addScriptToEvaluateOnNewDocument", { source: makeHostMockSource(scenario) });
  await send("Emulation.setDeviceMetricsOverride", { ...viewport, deviceScaleFactor: 1, mobile: false });

  const loaded = waitFor("Page.loadEventFired", 20_000);
  await send("Page.navigate", { url: serverUrl });
  await loaded;

  const page = {
    send,
    waitFor,
    evaluate: (expression) => evaluate({ send }, expression),
    async resize(width, height) {
      await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
      await delay(250);
    },
    async close() {
      await client.send("Target.closeTarget", { targetId: target.targetId }).catch(() => {});
    },
  };
  return page;
}

let serverUrl;
let server;

test("Browser layout, catalog diagnostics, and keyboard collapse on the current build", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; browser verification did not run and is not counted as passing");
    return;
  }
  assertFreshBuild();
  ({ url: serverUrl, server } = await startStaticServer());
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    const page = await openPage(session);
    try {
      // 页面挂载完成
      await waitForValue({ send: page.send }, `!!document.querySelector('main[aria-label="Plugin workspace"]')`, { label: "app mounted" });

      // 等待真实 SSH 前端 Shadow DOM 在工作区内完整挂载
      await waitForValue(
        { send: page.send },
        `(() => {
          const workspace = document.querySelector('main[aria-label="Plugin workspace"]');
          const hosts = workspace ? Array.from(workspace.querySelectorAll('div')) : [];
          return hosts.some((host) => !!(host.shadowRoot && host.shadowRoot.querySelector('.layout')));
        })()`,
        { label: "real SSH frontend Shadow DOM mounted in workspace" },
      );

      const takeScreenshot = async (name) => {
        const { data } = await page.send("Page.captureScreenshot", { format: "png" });
        const filePath = path.join(rootDir, "attachments", name);
        fs.mkdirSync(path.dirname(filePath), { recursive: true });
        fs.writeFileSync(filePath, Buffer.from(data, "base64"));
      };

      const measure = async () => page.evaluate(`(() => {
        const plugins = document.querySelector('aside[aria-label="Installed plugins"]');
        const instances = document.getElementById('instances-sidebar');
        const workspace = document.querySelector('main[aria-label="Plugin workspace"]');
        const toggle = document.getElementById('toggle-instances-button');
        const visible = (element) => !!element && getComputedStyle(element).display !== 'none' && element.getBoundingClientRect().width > 0;
        const rect = (element) => element ? element.getBoundingClientRect() : null;
        const p = rect(plugins), i = rect(instances), w = rect(workspace);
        return {
          innerWidth: window.innerWidth,
          pluginsWidth: p ? Math.round(p.width) : 0,
          pluginsRight: p ? Math.round(p.right) : 0,
          instancesVisible: visible(instances),
          instancesWidth: i ? Math.round(i.width) : 0,
          instancesLeft: i ? Math.round(i.left) : 0,
          instancesRight: i ? Math.round(i.right) : 0,
          workspaceWidth: w ? Math.round(w.width) : 0,
          workspaceLeft: w ? Math.round(w.left) : 0,
          scrollWidth: document.documentElement.scrollWidth,
          clientWidth: document.documentElement.clientWidth,
          ariaExpanded: toggle ? toggle.getAttribute('aria-expanded') : null,
          ariaControls: toggle ? toggle.getAttribute('aria-controls') : null,
          ariaLabel: toggle ? toggle.getAttribute('aria-label') : null,
        };
      })()`);

      // --- 1280px：两栏都展开，工作区拿到剩余宽度 ---
      await page.resize(1280, 800);
      const wide = await measure();
      assert.equal(wide.pluginsWidth, 72, "1280px: plugin rail is 72px");
      assert.equal(wide.instancesWidth, 240, "1280px: instances panel is 240px");
      assert.equal(wide.instancesVisible, true, "1280px: instances panel visible");
      assert.equal(wide.workspaceWidth, 968, "1280px: workspace takes the remaining 968px");
      assert.ok(wide.instancesLeft >= wide.pluginsRight, "1280px: no overlap between plugin rail and instances");
      assert.ok(wide.workspaceLeft >= wide.instancesRight, "1280px: no overlap between instances and workspace");
      assert.ok(wide.scrollWidth <= wide.clientWidth, `1280px: no horizontal overflow (${wide.scrollWidth} <= ${wide.clientWidth})`);
      await takeScreenshot("layout-1280px.png");

      // --- 1024px：实例栏收窄到 200px ---
      await page.resize(1024, 768);
      const medium = await measure();
      assert.equal(medium.pluginsWidth, 72, "1024px: plugin rail stays 72px");
      assert.equal(medium.instancesWidth, 200, "1024px: instances panel narrows to 200px");
      assert.equal(medium.workspaceWidth, 752, "1024px: workspace is 752px");
      assert.ok(medium.scrollWidth <= medium.clientWidth, `1024px: no horizontal overflow (${medium.scrollWidth} <= ${medium.clientWidth})`);
      assert.ok(medium.workspaceLeft >= medium.instancesRight, "1024px: workspace does not overlap instances");
      await takeScreenshot("layout-1024px.png");

      // --- 768px：低于 900px 断点，实例栏默认收起，工作区优先 ---
      await page.resize(768, 1024);
      await waitForValue({ send: page.send }, `document.getElementById('toggle-instances-button').getAttribute('aria-expanded') === 'false'`, { label: "auto-collapse below 900px" });
      const narrow = await measure();
      assert.equal(narrow.instancesVisible, false, "768px: instances panel auto-collapsed");
      assert.equal(narrow.workspaceWidth, 696, "768px: workspace gets 768 - 72 = 696px");
      assert.equal(narrow.ariaExpanded, "false", "768px: toggle reports aria-expanded=false");
      assert.equal(narrow.ariaControls, "instances-sidebar", "768px: toggle targets the instances panel");
      assert.equal(narrow.ariaLabel, "Expand instances panel", "768px: toggle has an accessible name for the expand action");
      assert.ok(narrow.scrollWidth <= narrow.clientWidth, `768px: no horizontal overflow (${narrow.scrollWidth} <= ${narrow.clientWidth})`);
      await takeScreenshot("layout-768px-collapsed.png");

      // --- 768px 纯键盘展开 ---
      await page.evaluate("document.getElementById('toggle-instances-button').focus()");
      assert.equal(await page.evaluate("document.activeElement.id"), "toggle-instances-button", "768px: toggle is focusable by keyboard");
      await page.send("Input.dispatchKeyEvent", {
        type: "rawKeyDown",
        key: "Enter",
        code: "Enter",
        windowsVirtualKeyCode: 13,
        nativeVirtualKeyCode: 13,
        text: "\r",
        unmodifiedText: "\r",
      });
      await page.send("Input.dispatchKeyEvent", {
        type: "char",
        key: "Enter",
        code: "Enter",
        windowsVirtualKeyCode: 13,
        nativeVirtualKeyCode: 13,
        text: "\r",
        unmodifiedText: "\r",
      });
      await page.send("Input.dispatchKeyEvent", {
        type: "keyUp",
        key: "Enter",
        code: "Enter",
        windowsVirtualKeyCode: 13,
        nativeVirtualKeyCode: 13,
      });
      await waitForValue({ send: page.send }, `document.getElementById('toggle-instances-button').getAttribute('aria-expanded') === 'true'`, { label: "keyboard expand" });
      const expanded = await measure();
      assert.equal(expanded.instancesVisible, true, "768px after Enter: instances panel expands");
      assert.equal(expanded.instancesWidth, 200, "768px after Enter: expanded panel is 200px");
      assert.equal(expanded.workspaceWidth, 496, "768px after Enter: workspace shrinks to 496px without overlap");
      assert.ok(expanded.workspaceLeft >= expanded.instancesRight, "768px after Enter: workspace does not overlap instances");
      assert.ok(expanded.scrollWidth <= expanded.clientWidth, `768px after Enter: no horizontal overflow (${expanded.scrollWidth} <= ${expanded.clientWidth})`);
      await takeScreenshot("layout-768px-expanded.png");

      // --- 768px 纯键盘再折叠（原生 Enter 激活） ---
      await page.evaluate("document.querySelector('#instances-sidebar button[aria-label=\"Collapse instances panel\"]').focus()");
      assert.equal(
        await page.evaluate("document.activeElement.getAttribute('aria-label')"),
        "Collapse instances panel",
        "768px: collapse button is focusable by keyboard",
      );
      await page.send("Input.dispatchKeyEvent", {
        type: "rawKeyDown",
        key: "Enter",
        code: "Enter",
        windowsVirtualKeyCode: 13,
        nativeVirtualKeyCode: 13,
        text: "\r",
        unmodifiedText: "\r",
      });
      await page.send("Input.dispatchKeyEvent", {
        type: "char",
        key: "Enter",
        code: "Enter",
        windowsVirtualKeyCode: 13,
        nativeVirtualKeyCode: 13,
        text: "\r",
        unmodifiedText: "\r",
      });
      await page.send("Input.dispatchKeyEvent", {
        type: "keyUp",
        key: "Enter",
        code: "Enter",
        windowsVirtualKeyCode: 13,
        nativeVirtualKeyCode: 13,
      });
      await waitForValue({ send: page.send }, `document.getElementById('toggle-instances-button').getAttribute('aria-expanded') === 'false'`, { label: "collapse again via keyboard" });
      const recollapsed = await measure();
      assert.equal(recollapsed.instancesVisible, false, "768px: collapse is reversible via keyboard");
      assert.equal(recollapsed.workspaceWidth, 696, "768px: workspace returns to 696px");

      // --- 逐包诊断与有效插件并存 ---
      const diagnostics = await page.evaluate(`(() => {
        const list = document.querySelector('ul[aria-label="Plugin package issues"]');
        return {
          present: !!list,
          entries: list ? [...list.querySelectorAll('li')].map((item) => item.textContent) : [],
          pluginButtons: [...document.querySelectorAll('aside[aria-label="Installed plugins"] nav button[aria-label^="Manage "]')].map((item) => item.getAttribute('aria-label')),
          roles: list ? list.getAttribute('role') : null,
        };
      })()`);
      assert.equal(diagnostics.present, true, "broken packages surface as a visible diagnostics list");
      assert.equal(diagnostics.roles, "status", "diagnostics use a status role, not an alert");
      assert.equal(diagnostics.entries.length, 2, "each broken package gets its own diagnostics entry");
      assert.match(diagnostics.entries[0], /broken-plugin/, "entry identifies the failing directory when pluginId is null");
      assert.match(diagnostics.entries[0], /not valid JSON/, "entry carries the user-facing message");
      assert.match(diagnostics.entries[1], /legacy-plugin/, "second broken package is listed too");
      assert.deepEqual(diagnostics.pluginButtons, ["Manage SSH"], "valid plugin stays listable alongside broken packages");

      // --- 真实 SSH 前端 Shadow DOM：计算样式与 token 继承 ---
      const sshStyles = await page.evaluate(`(() => {
        const workspace = document.querySelector('main[aria-label="Plugin workspace"]');
        const host = workspace ? Array.from(workspace.querySelectorAll('div')).find((d) => d.shadowRoot) : null;
        const root = host ? host.shadowRoot : null;
        const layout = root ? root.querySelector('.layout') : null;
        const bar = root ? root.querySelector('.bar') : null;
        const strong = root ? root.querySelector('.bar strong') : null;
        return {
          hostColor: host ? getComputedStyle(host).color : null,
          layoutBg: layout ? getComputedStyle(layout).backgroundColor : null,
          barBg: bar ? getComputedStyle(bar).backgroundColor : null,
          strongColor: strong ? getComputedStyle(strong).color : null,
          hasTerminal: !!(root && root.querySelector('.terminal')),
        };
      })()`);
      assert.equal(sshStyles.hostColor, "rgb(204, 204, 204)", "SSH :host inherits --coterm-text-primary");
      assert.equal(sshStyles.layoutBg, "rgb(30, 30, 30)", "SSH .layout background inherits --coterm-surface");
      assert.equal(sshStyles.barBg, "rgb(37, 37, 38)", "SSH .bar background inherits --coterm-surface-raised");
      assert.equal(sshStyles.strongColor, "rgb(204, 204, 204)", "SSH .bar strong color inherits --coterm-text-primary");
      assert.equal(sshStyles.hasTerminal, true, "SSH terminal container exists in real frontend");

      // --- SDK mountBlockList：实际组件计算样式与 fallback ---
      const blockStyles = await page.evaluate(`(() => {
        const container = document.createElement('div');
        document.body.append(container);
        window.__cotermTestMountBlockList(container, {
          blocks: [
            { id: "b1", index: 1, command: "whoami", actor: "robot", exitCode: 0 },
            { id: "b2", index: 2, command: "cat /etc/passwd", actor: "human", exitCode: 1 }
          ]
        });
        const root = container.querySelector('.coterm-blocks');
        const robotActor = container.querySelector('.coterm-block-actor[data-actor="robot"]');
        const status = container.querySelectorAll('.coterm-block-status')[1];

        // Also test an undeclared token fallback
        const fallbackEl = document.createElement('span');
        fallbackEl.style.cssText = 'color: var(--coterm-undefined-token, rgb(250, 204, 21));';
        container.append(fallbackEl);

        const result = {
          rootColor: getComputedStyle(root).color,
          rootBg: getComputedStyle(root).backgroundColor,
          robotColor: getComputedStyle(robotActor).color,
          statusColor: getComputedStyle(status).color,
          fallbackColor: getComputedStyle(fallbackEl).color,
        };
        container.remove();
        return result;
      })()`);
      assert.equal(blockStyles.rootColor, "rgb(204, 204, 204)", "SDK block-list inherits --coterm-text-primary");
      assert.equal(blockStyles.rootBg, "rgb(30, 30, 30)", "SDK block-list inherits --coterm-surface");
      assert.equal(blockStyles.robotColor, "rgb(117, 190, 255)", "SDK robot actor inherits --coterm-accent-glow");
      assert.equal(blockStyles.statusColor, "rgb(244, 135, 113)", "SDK failure status inherits --coterm-danger");
      assert.equal(blockStyles.fallbackColor, "rgb(250, 204, 21)", "missing tokens calculate declared fallback");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
    await new Promise((resolve) => server.close(resolve));
  }
});

test("Snapshot failure keeps the last valid attention state on screen", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; snapshot recovery was not exercised");
    return;
  }
  assertFreshBuild();
  const staticServer = await startStaticServer();
  serverUrl = staticServer.url;
  const scenario = {
    ...defaultScenario,
    attention: {
      version: 4,
      approvals: [{
        operationId: "op-1",
        proposalId: "proposal-1",
        pluginId: "ssh",
        pluginInstanceId: "inst-1",
        operation: "terminal.write",
        targetLabel: "Prod-Server",
        createdAt: "2026-10-22T00:00:00Z",
      }],
      interactions: [],
    },
    history: [{
      operationId: "op-1",
      pluginId: "ssh",
      pluginInstanceId: "inst-1",
      operation: "terminal.write",
      status: "awaiting_approval",
      proposalId: "proposal-1",
      input: { command: "ls -la" },
      createdAt: "2026-10-22T00:00:00Z",
    }],
  };
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });
  try {
    const page = await openPage(session, scenario);

    // 快照成功：审批卡片出现
    await waitForValue({ send: page.send }, `!!document.querySelector('form[aria-label="Agent operation approval"]')`, { label: "approval card rendered" });

    // 让下一次快照失败并触发一次 changed 事件
    await page.evaluate(`(() => { window.__cotermTest.setFailure('host_attention', true, [1]); window.__cotermTest.emitAttention(5); })()`);

    // 错误可见，但卡片保留（不清空待处理项）
    await waitForValue({ send: page.send }, `!!document.querySelector('main [role="alert"], aside [role="alert"]')`, { label: "recovery notice shown" });
    const afterFailure = await page.evaluate(`(() => ({
      cards: document.querySelectorAll('form[aria-label="Agent operation approval"]').length,
      alerts: [...document.querySelectorAll('[role="alert"]')].map((item) => item.textContent).join(' | '),
    }))()`);
    assert.equal(afterFailure.cards, 1, "failed snapshot keeps the last valid approval list on screen");
    assert.match(afterFailure.alerts, /Unable to follow Agent approvals/, "failure is reported as a recoverable notice");

    // 恢复：下一次快照成功后错误提示消失，状态继续跟随
    await page.evaluate(`new Promise((resolve) => {
      setTimeout(() => {
        window.__cotermTest.emitAttention(6);
        resolve();
      }, 50);
    })`);
    await waitForValue({ send: page.send }, `!document.querySelector('[role="alert"]')`, { label: "notice cleared after recovery" });
    const recovered = await page.evaluate(`document.querySelectorAll('form[aria-label="Agent operation approval"]').length`);
    assert.equal(recovered, 1, "attention subscription resumes after a transient failure");

    // 宿主收敛（审批已处理）：卡片随快照消失
    await page.evaluate(`window.__cotermTest.setAttention({ version: 7, approvals: [], interactions: [] })`);
    await waitForValue({ send: page.send }, `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 0`, { label: "card removed on convergence" });
  } finally {
    await session.close();
    await new Promise((resolve) => staticServer.server.close(resolve));
  }
});

test("Multi-card independent approvals and UI convergence in the browser", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; approvals verification skipped");
    return;
  }
  assertFreshBuild();
  const staticServer = await startStaticServer();
  serverUrl = staticServer.url;
  const multiScenario = {
    ...defaultScenario,
    attention: {
      version: 1,
      approvals: [
        {
          operationId: "op-inst-1",
          proposalId: "prop-1",
          pluginId: "ssh",
          pluginInstanceId: "inst-1",
          operation: "terminal.write",
          targetLabel: "Web-Server",
          createdAt: "2026-10-22T00:00:00Z",
        },
        {
          operationId: "op-inst-2",
          proposalId: "prop-2",
          pluginId: "ssh",
          pluginInstanceId: "inst-2",
          operation: "terminal.write",
          targetLabel: "DB-Server",
          createdAt: "2026-10-22T00:01:00Z",
        },
        {
          operationId: "op-plugin-1",
          proposalId: "prop-3",
          pluginId: "ssh",
          pluginInstanceId: null,
          operation: "ssh.scan",
          targetLabel: "SSH Plugin",
          createdAt: "2026-10-22T00:02:00Z",
        },
      ],
      interactions: [],
    },
    history: [
      {
        operationId: "op-inst-1",
        pluginId: "ssh",
        pluginInstanceId: "inst-1",
        operation: "terminal.write",
        status: "awaiting_approval",
        proposalId: "prop-1",
        input: { command: "systemctl restart nginx" },
      },
      {
        operationId: "op-inst-2",
        pluginId: "ssh",
        pluginInstanceId: "inst-2",
        operation: "terminal.write",
        status: "awaiting_approval",
        proposalId: "prop-2",
        input: { command: "journalctl -u nginx" },
      },
      {
        operationId: "op-plugin-1",
        pluginId: "ssh",
        operation: "ssh.scan",
        status: "awaiting_approval",
        proposalId: "prop-3",
        input: { action: "scan" },
      },
    ],
  };

  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });
  try {
    const page = await openPage(session, multiScenario);
    try {
      // 3 张卡片均正确显示
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 3`,
        { label: "all 3 approval cards rendered" },
      );

      // 用户批准第一张卡片（inst-1）
      await page.evaluate(`(() => {
        const cards = document.querySelectorAll('form[aria-label="Agent operation approval"]');
        const submit = cards[0].querySelector('button[type="submit"]');
        submit.click();
      })()`);

      // 剩余 2 张卡片保留在屏幕上
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 2`,
        { label: "2 approval cards remaining after card 1 approved" },
      );

      // 用户拒绝插件级卡片（op-plugin-1，当前第 2 张）
      await page.evaluate(`(() => {
        const cards = document.querySelectorAll('form[aria-label="Agent operation approval"]');
        const reject = cards[1].querySelector('button[type="button"]');
        reject.click();
      })()`);

      // 剩余 1 张卡片
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 1`,
        { label: "1 approval card remaining after card 3 rejected" },
      );

      // 用户批准剩余卡片
      await page.evaluate(`(() => {
        const cards = document.querySelectorAll('form[aria-label="Agent operation approval"]');
        const submit = cards[0].querySelector('button[type="submit"]');
        submit.click();
      })()`);

      // 验证收敛：全部卡片消失
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 0`,
        { label: "all cards cleared on convergence" },
      );
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
    await new Promise((resolve) => staticServer.server.close(resolve));
  }
});
