/**
 * 当前构建的真实浏览器验收：布局（1280/1024/768）、纯键盘折叠恢复、DriverCatalog 逐包诊断、
 * 真实宿主 SSH 工作台与 React mountBlockList 的主题计算样式验证、多审批卡片独立处理。
 *
 * 这里验证的是 App.tsx + index.css + 宿主工作台 的实际浏览器渲染结果。
 * 没有浏览器时整组 skip 并写明原因；skip 不等于通过。
 */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  setAppLocale,
  assertFreshBuild,
  assertFreshTestArtifacts,
  delay,
  distDir,
  evaluate,
  findBrowser,
  openSession,
  rootDir,
  startStaticServer,
  waitForValue,
} from "#support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "#support/host-mock.js";

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

test("SSH terminal bottom row stays inside the workspace above the host footer", async (t) => {
  if (!browserPath) { t.skip("No browser; SSH bottom row was not verified"); return; }
  assertFreshBuild();
  const staticServer = await startStaticServer();
  serverUrl = staticServer.url;
  const session = await openSession({ browserPath, totalTimeoutMs: 45_000 });
  let page;
  try {
    page = await openPage(session, {
      ...defaultScenario,
      catalog: { ...defaultScenario.catalog, issues: [] },
      sshOutput: Array.from({ length: 100 }, (_, i) => `output line ${i}\r\n`).join("") + "BOTTOM ROW VISIBLE > ",
    });
    await waitForValue({ send: page.send }, `(() => {
      window.sshHost = [...document.querySelectorAll('main div')].find(node => node?.querySelector('.xterm-screen'));
      window.sshRoot = sshHost;
      return sshRoot?.querySelector('.bar span')?.textContent === 'Connected';
    })()`);
    for (const [width, height, fontSize] of [[1280, 800, 13], [1024, 600, 20], [768, 480, 16]]) {
      await page.evaluate(`document.documentElement.style.setProperty('--coterm-terminal-font-size', '${fontSize}px')`);
      await page.resize(width, height);
      const geometry = await page.evaluate(`(() => {
        const rect = element => { const r=element.getBoundingClientRect(); return {top:r.top,bottom:r.bottom,height:r.height}; };
        return {
          viewport:innerHeight,
          footer:rect(document.querySelector('footer')),
          workspace:rect(sshHost.closest('[role="tabpanel"]')),
          terminal:rect(sshRoot.querySelector('.terminal')),
          screen:rect(sshRoot.querySelector('.xterm-screen')),
        };
      })()`);
      const shot = await page.send("Page.captureScreenshot", { format: "png" });
      fs.mkdirSync(path.join(rootDir, ".agent-debug", "layout"), { recursive: true });
      fs.writeFileSync(path.join(rootDir, ".agent-debug", "layout", `ssh-bottom-layout-${width}x${height}.png`), Buffer.from(shot.data, "base64"));
      assert.ok(geometry.screen.height > 0, "real terminal rows are rendered");
      assert.equal(await page.evaluate(`sshRoot.querySelector('.xterm-rows').lastElementChild.textContent.includes('BOTTOM ROW VISIBLE')`), true,
        "the prompt is rendered on the final terminal row after scrolling");
      assert.ok(geometry.footer.bottom <= height + 1, "host footer stays inside the window");
      assert.ok(geometry.workspace.bottom <= geometry.footer.top + 1, "workspace stays above the host footer");
      assert.ok(geometry.screen.bottom <= geometry.terminal.bottom + 1,
        `the terminal's last row must not be clipped by its container at ${width}x${height}: ${JSON.stringify(geometry)}`);
      assert.ok(geometry.screen.bottom <= geometry.footer.top + 1,
        "the terminal's last row stays above the host footer");
    }
  } finally {
    await page?.close();
    await session.close();
    await new Promise(resolve => staticServer.server.close(resolve));
  }
});

test("appearance switches live SSH and serial renderers, persists, and follows system changes", async (t) => {
  if (!browserPath) { t.skip("No Chrome/Edge binary; live theme verification did not run"); return; }
  assertFreshBuild();
  const staticServer = await startStaticServer();
  serverUrl = staticServer.url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });
  let page;
  try {
    page = await openPage(session);
    await waitForValue({ send: page.send }, `(() => {
      const host = [...document.querySelectorAll('main div')].find(node => node?.querySelector('.xterm'));
      if (!host || !host.querySelector('.bar span')?.textContent.includes('Connected')) return false;
      window.themeSsh = host; window.themeTerminal = host.querySelector('.xterm');
      window.themeSubscriptions = window.__cotermTest.calls.filter(c => c === 'subscribe_driver_events').length;
      window.themeConnectionCalls = window.__cotermTest.invocations.filter(c => c.command === 'invoke_driver_operation' && ['ssh.connect','ssh.disconnect','ssh.write'].includes(c.args.operation)).length;
      return true;
    })()`, { label: "SSH ready for live appearance switch" });
    const choose = async (mode) => {
      await page.evaluate(`document.querySelector('#theme-switcher button[data-state][aria-label=${JSON.stringify({ light: 'Light mode', dark: 'Dark mode', system: 'System theme' }[mode])}]').click()`);
      await waitForValue({ send: page.send }, `document.querySelector('#theme-switcher button[aria-label=${JSON.stringify({ light: 'Light mode', dark: 'Dark mode', system: 'System theme' }[mode])}]')?.getAttribute('data-state') === 'on'`);
    };
    const screenshot = async (name) => {
      await page.evaluate(`Promise.allSettled(document.getAnimations().filter(animation => animation.effect?.getTiming().iterations !== Infinity).map(animation => animation.finished))`);
      const result = await page.send("Page.captureScreenshot", { format: "png" });
      fs.mkdirSync(path.join(rootDir, ".agent-debug", "layout"), { recursive: true });
      fs.writeFileSync(path.join(rootDir, ".agent-debug", "layout", name), Buffer.from(result.data, "base64"));
    };
    const terminalBackground = (host) => page.evaluate(`getComputedStyle(${host}.querySelector('.xterm-viewport')).backgroundColor`);
    await choose("dark");
    await waitForValue({ send: page.send }, `document.documentElement.dataset.theme === 'dark'`);
    assert.equal(await terminalBackground("themeSsh"), "rgb(21, 26, 35)");
    await screenshot("desktop-theme-dark.png");
    await choose("light");
    await waitForValue({ send: page.send }, `document.documentElement.dataset.theme === 'light'`);
    assert.equal(await terminalBackground("themeSsh"), "rgb(255, 255, 255)");
    assert.equal(await page.evaluate(`themeTerminal === themeSsh.querySelector('.xterm')`), true);
    assert.equal(await page.evaluate(`window.__cotermTest.calls.filter(c => c === 'subscribe_driver_events').length === themeSubscriptions`), true, "theme changes do not remount the SSH driver");
    assert.equal(await page.evaluate(`window.__cotermTest.invocations.filter(c => c.command === 'invoke_driver_operation' && ['ssh.connect','ssh.disconnect','ssh.write'].includes(c.args.operation)).length === themeConnectionCalls`), true);
    for (const [width, height] of [[1280, 800], [1024, 768], [768, 800]]) {
      await page.resize(width, height);
      assert.equal(await page.evaluate(`document.documentElement.scrollWidth <= innerWidth`), true);
      assert.equal(await page.evaluate(`(() => { const r = document.getElementById('theme-switcher').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth; })()`), true);
      await screenshot(`desktop-theme-light-${width}.png`);
    }
    await page.resize(1280, 800);

    // Mount the actual serial bundle in the same themed document, with a typed-contract mock only.
    const artifact = path.join(rootDir, "apps/desktop/dist/workbenches/serial/index.js");
    assert.ok(fs.statSync(artifact).mtimeMs >= fs.statSync(path.join(rootDir, "apps/desktop/src/workbenches/common/terminal-theme.ts")).mtimeMs);
    const serialURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
    await page.evaluate(`(async () => {
      const container = document.createElement('div'); container.style.cssText = 'position:fixed;inset:80px 24px 50px 330px'; document.body.append(container);
      window.themeSerialCalls = []; window.themeSerialSubscriptions = 0;
      const receipt = output => ({ operationId:'theme',status:'completed',output,error:null,proposalId:null,waitExpired:false });
      const api = { driverInstanceId:'theme-serial', async invoke(operation,args) {
        themeSerialCalls.push({operation,args});
        if (operation === 'serial.status') return receipt({config:{port:'COM1',baudRate:115200,dataBits:8,stopBits:1,parity:'none',dtr:true,rts:true},connected:true,connectionId:'theme-conn',signals:{dtr:true,rts:true},rxBytes:0,txBytes:0,commandQueue:0});
        if (operation === 'serial.read') return receipt({text:'Theme probe\\r\\n',bytes:13,cursor:1,gap:false,closed:false,more:false,connectionId:'theme-conn'});
        throw new Error('Unexpected operation: ' + operation);
      }, async subscribeEvents() { themeSerialSubscriptions++; return () => {}; } };
      window.themeSerialFrontend = (await import(${JSON.stringify(serialURL)})).mount(container, api);
      window.themeSerial = container.firstElementChild;
      window.themeSerialTerminal = themeSerial.querySelector('.xterm');
    })()`);
    await waitForValue({ send: page.send }, `themeSerial.querySelector('.connect-button')?.dataset.connected === 'true'`);
    assert.equal(await terminalBackground("themeSerial"), "rgb(255, 255, 255)");
    await choose("dark");
    await waitForValue({ send: page.send }, `getComputedStyle(themeSerial.querySelector('.xterm-viewport')).backgroundColor === 'rgb(21, 26, 35)'`);
    assert.equal(await terminalBackground("themeSsh"), "rgb(21, 26, 35)");
    assert.equal(await page.evaluate(`themeSerialTerminal === themeSerial.querySelector('.xterm') && themeSerialSubscriptions === 1`), true);
    assert.equal(await page.evaluate(`themeSerialCalls.every(c => ['serial.status','serial.read'].includes(c.operation))`), true, "palette changes never send serial bytes or reconnect");
    assert.equal(await page.evaluate(`themeSerial.querySelector('.xterm-screen').textContent.includes('Theme probe')`), true, "existing terminal output survives");
    await screenshot("desktop-theme-serial-dark.png");
    await page.evaluate(`(() => { const container = themeSerial.parentElement; themeSerialFrontend.unmount(); container.remove(); })()`);

    // Explicit modes override OS changes; system mode responds immediately.
    await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
    assert.equal(await page.evaluate(`document.documentElement.dataset.theme`), "dark");
    await page.send("Page.reload");
    await waitForValue({ send: page.send }, `document.getElementById('theme-switcher') && document.documentElement.dataset.theme === 'dark'`);
    assert.equal(await page.evaluate(`localStorage.getItem('coterm.theme')`), "dark");
    await choose("system");
    await waitForValue({ send: page.send }, `document.documentElement.dataset.theme === 'light'`);
    await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
    await waitForValue({ send: page.send }, `document.documentElement.dataset.theme === 'dark'`);
    await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
    await waitForValue({ send: page.send }, `document.documentElement.dataset.theme === 'light'`);
  } finally {
    await page?.close();
    await session.close();
    await new Promise(resolve => staticServer.server.close(resolve));
  }
});

test("Language switching and built-in driver settings preserve live SSH state", async (t) => {
  if (!browserPath) { t.skip("No Chrome/Edge binary found; alpha browser verification did not run"); return; }
  assertFreshBuild();
  const staticServer = await startStaticServer();
  serverUrl = staticServer.url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });
  let page;
  try {
    page = await openPage(session, defaultScenario);
    await waitForValue({ send: page.send }, `(() => {
      const host = [...document.querySelectorAll('main div')].find((node) => node?.querySelector('.layout'));
      if (!host || host.querySelector('.bar span')?.textContent !== 'Connected') return false;
      window.__alphaSshHost = host;
      window.__alphaTerminal = host.querySelector('.xterm');
      window.__alphaSubscriptions = window.__cotermTest.calls.filter((command) => command === 'subscribe_driver_events').length;
      return !!window.__alphaTerminal;
    })()`, { label: "live SSH frontend" });
    const switchLanguage = (locale) => setAppLocale({ send: page.send }, locale);
    await switchLanguage("zh-CN");
    await waitForValue({ send: page.send }, `document.documentElement.lang === 'zh-CN' && !document.getElementById('install-driver-button')`, { label: "Chinese host controls" });
    assert.equal(await page.evaluate(`window.__alphaSshHost.querySelector('.bar span').textContent`), "已连接");
    assert.equal(await page.evaluate(`window.__alphaTerminal === window.__alphaSshHost.querySelector('.xterm')`), true);
    assert.equal(await page.evaluate(`window.__cotermTest.countCalls('subscribe_driver_events') === window.__alphaSubscriptions`), true);
    await page.evaluate(`document.getElementById('open-settings').click()`);
    await waitForValue({ send: page.send }, `document.getElementById('app-settings-dialog')?.open`);
    assert.equal(await page.evaluate(`!!document.querySelector('[data-category="drivers"]')`),false);
    assert.equal(await page.evaluate(`!!document.getElementById('install-driver-package')`),false);
    assert.equal(await page.evaluate(`window.__alphaTerminal===window.__alphaSshHost.querySelector('.xterm')`),true);
    await page.evaluate(`document.querySelector('#app-settings-dialog button[aria-label="关闭对话框"]').click()`);
    await page.send("Page.reload");
    await waitForValue({ send: page.send }, `document.documentElement.lang === 'zh-CN' && !!document.getElementById('open-settings')`, { label: "language preference survives reload" });
    await switchLanguage("en");
    await waitForValue({ send: page.send }, `document.documentElement.lang==='en' && !document.getElementById('install-driver-button')`, { label: "switch back to English" });
  } finally {
    await page?.close();
    await session.close();
    await new Promise((resolve) => staticServer.server.close(resolve));
  }
});

test("Browser layout, catalog diagnostics, and keyboard collapse on the current build", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; browser verification did not run and is not counted as passing");
    return;
  }
  assertFreshBuild();
  assertFreshTestArtifacts();
  ({ url: serverUrl, server } = await startStaticServer());
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    const page = await openPage(session);
    try {
      // 页面挂载完成
      await waitForValue({ send: page.send }, `!!document.querySelector('main[aria-label="Driver workspace"]')`, { label: "app mounted" });

      // 等待真实 SSH 工作台在工作区内完整挂载
      await waitForValue(
        { send: page.send },
        `(() => {
          const workspace = document.querySelector('main[aria-label="Driver workspace"]');
          const hosts = workspace ? Array.from(workspace.querySelectorAll('div')) : [];
          return hosts.some((host) => !!(host && host.querySelector('.layout')));
        })()`,
        { label: "real SSH host workbench mounted in workspace" },
      );

      const takeScreenshot = async (name) => {
        const { data } = await page.send("Page.captureScreenshot", { format: "png" });
        const filePath = path.join(rootDir, ".agent-debug", "layout", name);
        fs.mkdirSync(path.dirname(filePath), { recursive: true });
        fs.writeFileSync(filePath, Buffer.from(data, "base64"));
      };

      const measure = async () => page.evaluate(`(() => {
        const drivers = document.querySelector('aside[aria-label="Built-in drivers"]');
        const instances = document.getElementById('instances-sidebar');
        const workspace = document.querySelector('main[aria-label="Driver workspace"]');
        const toggle = document.getElementById('toggle-instances-button');
        const visible = (element) => !!element && getComputedStyle(element).display !== 'none' && element.getBoundingClientRect().width > 0;
        const rect = (element) => element ? element.getBoundingClientRect() : null;
        const p = rect(drivers), i = rect(instances), w = rect(workspace);
        return {
          innerWidth: window.innerWidth,
          driversWidth: p ? Math.round(p.width) : 0,
          driversRight: p ? Math.round(p.right) : 0,
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
      assert.equal(wide.driversWidth, 72, "1280px: driver rail is 72px");
      assert.equal(wide.instancesWidth, 240, "1280px: instances panel is 240px");
      assert.equal(wide.instancesVisible, true, "1280px: instances panel visible");
      assert.equal(wide.workspaceWidth, 968, "1280px: workspace takes the remaining 968px");
      assert.ok(wide.instancesLeft >= wide.driversRight, "1280px: no overlap between driver rail and instances");
      assert.ok(wide.workspaceLeft >= wide.instancesRight, "1280px: no overlap between instances and workspace");
      assert.ok(wide.scrollWidth <= wide.clientWidth, `1280px: no horizontal overflow (${wide.scrollWidth} <= ${wide.clientWidth})`);
      await takeScreenshot("layout-1280px.png");

      // --- 1024px：实例栏收窄到 200px ---
      await page.resize(1024, 768);
      const medium = await measure();
      assert.equal(medium.driversWidth, 72, "1024px: driver rail stays 72px");
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

      // --- 逐包诊断与有效驱动并存 ---
      const diagnostics = await page.evaluate(`(() => {
        const list = document.querySelector('ul[aria-label="Driver registration issues"]');
        return {
          present: !!list,
          entries: list ? [...list.querySelectorAll('li')].map((item) => item.textContent) : [],
          driverButtons: [...document.querySelectorAll('aside[aria-label="Built-in drivers"] nav button[aria-label^="Manage "]')].map((item) => item.getAttribute('aria-label')),
          roles: list ? list.getAttribute('role') : null,
        };
      })()`);
      assert.equal(diagnostics.present, true, "broken packages surface as a visible diagnostics list");
      assert.equal(diagnostics.roles, "status", "diagnostics use a status role, not an alert");
      assert.equal(diagnostics.entries.length, 2, "each broken package gets its own diagnostics entry");
      assert.match(diagnostics.entries[0], /broken-driver/, "entry identifies the failing directory when driverId is null");
      assert.match(diagnostics.entries[0], /not valid JSON/, "entry carries the user-facing message");
      assert.match(diagnostics.entries[1], /legacy-driver/, "second broken package is listed too");
      assert.deepEqual(diagnostics.driverButtons, ["Manage SSH"], "valid driver stays listable alongside broken packages");

      // --- 真实 SSH 工作台：计算样式与 token 继承 ---
      const palette = await page.evaluate(`(() => {
        const probe = document.createElement('span'); document.body.append(probe);
        const color = (token) => { probe.style.color = 'var(' + token + ')'; return getComputedStyle(probe).color; };
        const result = { text: color('--coterm-text-primary'), surface: color('--coterm-surface'), raised: color('--coterm-surface-raised'), accent: color('--coterm-accent-glow') };
        probe.remove(); return result;
      })()`);
      const sshStyles = await page.evaluate(`(() => {
        const workspace = document.querySelector('main[aria-label="Driver workspace"]');
        const host = workspace?.querySelector('[data-workbench=ssh]');
        const root = host ? host : null;
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
      assert.equal(sshStyles.hostColor, palette.text, "SSH :host inherits --coterm-text-primary");
      assert.equal(sshStyles.layoutBg, palette.surface, "SSH .layout background inherits --coterm-surface");
      assert.equal(sshStyles.barBg, palette.raised, "SSH .bar background inherits --coterm-surface-raised");
      assert.equal(sshStyles.strongColor, palette.text, "SSH .bar strong color inherits --coterm-text-primary");
      assert.equal(sshStyles.hasTerminal, true, "SSH terminal container exists in real frontend");

      await page.evaluate(`document.documentElement.style.setProperty('--radius','1.125rem')`);
      assert.equal(await page.evaluate(`(()=>{const host=document.querySelector('[data-workbench=ssh]');const probe=document.createElement('div');host.append(probe);probe.style.borderRadius='var(--radius-md)';const buttonRadius=getComputedStyle(probe).borderRadius;probe.style.borderRadius='var(--radius-sm)';const inputRadius=getComputedStyle(probe).borderRadius;probe.remove();return getComputedStyle(host.querySelector('.coterm-terminal-clear')).borderRadius===buttonRadius&&getComputedStyle(host.querySelector('.terminal-find input')).borderRadius===inputRadius})()`),true, "workbench buttons and inputs follow a changed root radius");
      await page.evaluate(`document.documentElement.style.removeProperty('--radius')`);

      // One theme source updates the mounted terminal without remounting it.
      await page.evaluate(`window.keptTerminal=document.querySelector('[data-workbench=ssh] .xterm');document.documentElement.classList.add('dark')`);
      assert.equal(await page.evaluate(`(()=>{const root=document.querySelector('[data-workbench=ssh]');const value=getComputedStyle(root).getPropertyValue('--coterm-surface').trim();const probe=document.createElement('div');probe.style.background=value;root.append(probe);const expected=getComputedStyle(probe).backgroundColor;probe.remove();return getComputedStyle(root.querySelector('.layout')).backgroundColor===expected&&root.querySelector('.xterm')===keptTerminal})()`),true);
      await page.evaluate(`document.documentElement.classList.remove('dark')`);

      // --- 真实共享前端 mountBlockList：token 继承、选择状态与销毁 ---
      // 组件来自 apps/desktop/src/workbenches/common/block-list.tsx 的构建产物，
      // 页面以动态 import 加载，脚本里没有任何手写 DOM 或样式替身。
      await page.evaluate(`(async () => { window.__sharedModule = await window.__cotermTest.loadShared(); })()`);
      await page.evaluate(`window.blockContainer=document.createElement('div');document.body.append(blockContainer);window.blockSelections=[];
        window.blockView=__sharedModule.mountBlockList(blockContainer,{blocks:[
          {id:'b1',index:1,command:'whoami',actor:'robot',exitCode:0},
          {id:'b2',index:2,command:'cat /etc/passwd',actor:'human',exitCode:null},
          {id:'b3',index:3,command:'false',actor:'robot',exitCode:1}
        ],onSelectionChange:ids=>blockSelections.push(ids)});void 0`);
      await waitForValue({send:page.send},`blockContainer.querySelectorAll('.coterm-block').length===3`);
      const blockStyles=await page.evaluate(`(()=>{const root=blockContainer.querySelector('.coterm-blocks');const rows=[...blockContainer.querySelectorAll('.coterm-block')];return {
        rowTagNames:rows.map(row=>row.tagName),rootRole:root.getAttribute('role'),rootLabel:root.getAttribute('aria-label'),
        rootColor:getComputedStyle(root).color,rootBg:getComputedStyle(root).backgroundColor,
        robotColor:getComputedStyle(blockContainer.querySelector('[data-actor=robot]')).color,
        actors:[...blockContainer.querySelectorAll('.coterm-block-actor')].map(node=>node.textContent),
        failureMark:rows[2].querySelector('.coterm-block-status').textContent,failureTitle:rows[2].querySelector('.coterm-block-status').title,
        successMark:rows[0].querySelector('.coterm-block-status').textContent
      }})()`);
      assert.deepEqual(blockStyles.rowTagNames,['ARTICLE','ARTICLE','ARTICLE']);
      assert.equal(blockStyles.rootRole,'list');assert.equal(blockStyles.rootLabel,'Command blocks');
      assert.equal(blockStyles.rootColor,palette.text);assert.equal(blockStyles.rootBg,palette.surface);assert.equal(blockStyles.robotColor,palette.accent);
      assert.deepEqual(blockStyles.actors,['Robot','Human','Robot']);assert.equal(blockStyles.failureMark,'!');assert.equal(blockStyles.failureTitle,'Exit code 1');assert.equal(blockStyles.successMark,'');
      await page.evaluate(`blockContainer.querySelectorAll('.coterm-block-gutter')[1].click()`);
      await waitForValue({send:page.send},`blockContainer.querySelectorAll('.coterm-block-gutter')[1].getAttribute('aria-pressed')==='true'`);
      assert.deepEqual(await page.evaluate(`blockSelections`),[['b2']]);
      await page.evaluate(`blockView.update({blocks:[{id:'b1',index:1,command:'whoami',actor:'robot',exitCode:0},{id:'b2',index:2,command:'cat /etc/passwd',actor:'human',selected:true}]})`);
      await waitForValue({send:page.send},`blockContainer.querySelectorAll('.coterm-block').length===2`);
      assert.deepEqual(await page.evaluate(`[...blockContainer.querySelectorAll('.coterm-block-gutter')].map(node=>node.getAttribute('aria-pressed'))`),['false','true']);
      await page.evaluate(`blockView.unmount();blockView.update({blocks:[{id:'late',index:9,command:'late',actor:'human'}]})`);
      await waitForValue({send:page.send},`blockContainer.childElementCount===0`);
      await page.evaluate(`blockContainer.remove()`);
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
  assertFreshTestArtifacts();
  const staticServer = await startStaticServer();
  serverUrl = staticServer.url;
  const scenario = {
    ...defaultScenario,
    attention: {
      version: 4,
      approvals: [{
        operationId: "op-1",
        proposalId: "proposal-1",
        driverId: "ssh",
        driverInstanceId: "inst-1",
        operation: "terminal.write",
        targetLabel: "Prod-Server",
        createdAt: "2026-10-22T00:00:00Z",
      }],
      interactions: [],
    },
    history: [{
      operationId: "op-1",
      driverId: "ssh",
      driverInstanceId: "inst-1",
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
    await waitForValue({ send: page.send }, `[...document.querySelectorAll('[role="alert"]')].every(node=>!node.textContent.trim()||!!node.closest('[hidden]'))`, { label: "notice cleared after recovery" });
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
  assertFreshTestArtifacts();
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
          driverId: "ssh",
          driverInstanceId: "inst-1",
          operation: "terminal.write",
          targetLabel: "Web-Server",
          createdAt: "2026-10-22T00:00:00Z",
        },
        {
          operationId: "op-inst-2",
          proposalId: "prop-2",
          driverId: "ssh",
          driverInstanceId: "inst-2",
          operation: "terminal.write",
          targetLabel: "DB-Server",
          createdAt: "2026-10-22T00:01:00Z",
        },
        {
          operationId: "op-driver-1",
          proposalId: "prop-3",
          driverId: "ssh",
          driverInstanceId: null,
          operation: "ssh.scan",
          targetLabel: "SSH Driver",
          createdAt: "2026-10-22T00:02:00Z",
        },
      ],
      interactions: [],
    },
    history: [
      {
        operationId: "op-inst-1",
        driverId: "ssh",
        driverInstanceId: "inst-1",
        operation: "terminal.write",
        status: "awaiting_approval",
        proposalId: "prop-1",
        input: { command: "systemctl restart nginx" },
      },
      {
        operationId: "op-inst-2",
        driverId: "ssh",
        driverInstanceId: "inst-2",
        operation: "terminal.write",
        status: "awaiting_approval",
        proposalId: "prop-2",
        input: { command: "journalctl -u nginx" },
      },
      {
        operationId: "op-driver-1",
        driverId: "ssh",
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
      // 队列默认折叠：只显示当前卡片，其他提案保持在固定高度的队列中。
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 1`,
        { label: "active approval card rendered" },
      );

      // 用户批准第一张卡片（inst-1）——按卡片自身的 targetLabel 定位，避免依赖 DOM 顺序。
      const cardTargets = async () => page.evaluate(`[...document.querySelectorAll('form[aria-label="Agent operation approval"]')]
        .map((card) => card.querySelector('[id$="-target"]').textContent)`);
      const initialTargets = await cardTargets();
      assert.equal(initialTargets.length, 1, "only the active card renders while the queue is collapsed");
      assert.ok(initialTargets[0].includes("Web-Server"), "card 1 identifies Web-Server");

      const queueSummary = await page.evaluate(`({
        pending: document.querySelector('[aria-label="Agent approvals"]').textContent,
        expanded: document.querySelector('[aria-label="Agent approvals"] button[aria-expanded]')?.getAttribute('aria-expanded'),
      })`);
      assert.match(queueSummary.pending, /3 pending/);
      assert.match(queueSummary.pending, /2 queued/);
      assert.equal(queueSummary.expanded, "false", "approval queue starts collapsed");

      await page.evaluate(`document.querySelector('[aria-label="Agent approvals"] button[aria-expanded]').click()`);
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 3`,
        { label: "expanded approval queue renders all cards" },
      );
      const expandedTargets = await cardTargets();
      assert.equal(expandedTargets.length, 3, "expanded queue renders all pending cards");

      await page.evaluate(`(() => {
        const cards = [...document.querySelectorAll('form[aria-label="Agent operation approval"]')];
        const card = cards.find((item) => item.querySelector('[id$="-target"]').textContent.includes("Web-Server"));
        card.querySelector('button[type="submit"]').click();
      })()`);

      // 剩余 2 张卡片保留在屏幕上，且身份是未决的那两张。
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 2`,
        { label: "2 approval cards remaining after card 1 approved" },
      );
      const afterApprove = await cardTargets();
      assert.ok(afterApprove.some((t) => t.includes("DB-Server")), "DB-Server card survives the first approval");
      assert.ok(afterApprove.some((t) => t.includes("SSH Driver")), "driver-level card survives the first approval");
      assert.ok(!afterApprove.some((t) => t.includes("Web-Server")), "the decided card is gone");

      // 批准必须打到真实的 command + 参数 + 提案身份，并被未决卡片保留验证。
      const approveCalls = await page.evaluate(`window.__cotermTest.approveCalls`);
      assert.equal(approveCalls.length, 1, "exactly one approve command issued");
      assert.equal(approveCalls[0].command, "approve_proposal", "instance-level card uses approve_proposal");
      assert.equal(approveCalls[0].driverInstanceId, "inst-1", "approve targets the card's own instance");
      assert.equal(approveCalls[0].proposalId, "prop-1", "approve carries the card's proposal id");
      assert.deepEqual(approveCalls[0].args, { command: "systemctl restart nginx" }, "approve carries the edited draft args");

      // 用户拒绝驱动级卡片（SSH Driver）——按身份定位，不按索引。
      await page.evaluate(`(() => {
        const cards = [...document.querySelectorAll('form[aria-label="Agent operation approval"]')];
        const card = cards.find((item) => item.querySelector('[id$="-target"]').textContent.includes("SSH Driver"));
        card.querySelector('button[type="button"]').click();
      })()`);

      // 剩余 1 张卡片，且必须是 DB-Server。
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 1`,
        { label: "1 approval card remaining after the driver card is rejected" },
      );
      const afterReject = await cardTargets();
      assert.equal(afterReject.length, 1, "exactly one card remains");
      assert.ok(afterReject[0].includes("DB-Server"), "the surviving card is the still-pending DB-Server card");

      const rejectCalls = await page.evaluate(`window.__cotermTest.rejectCalls`);
      assert.equal(rejectCalls.length, 1, "exactly one reject command issued");
      assert.equal(rejectCalls[0].command, "reject_driver_proposal", "driver-level card uses reject_driver_proposal");
      assert.equal(rejectCalls[0].driverId, "ssh", "driver-level reject targets the driver id");
      assert.equal(rejectCalls[0].proposalId, "prop-3", "driver-level reject carries its proposal id");

      // 用户批准剩余卡片。
      await page.evaluate(`(() => {
        const cards = [...document.querySelectorAll('form[aria-label="Agent operation approval"]')];
        cards[0].querySelector('button[type="submit"]').click();
      })()`);

      // 验证收敛：全部卡片消失，且最后一次批准打的是 DB-Server。
      await waitForValue(
        { send: page.send },
        `document.querySelectorAll('form[aria-label="Agent operation approval"]').length === 0`,
        { label: "all cards cleared on convergence" },
      );
      const finalApproves = await page.evaluate(`window.__cotermTest.approveCalls`);
      assert.equal(finalApproves.length, 2, "two approvals in total, one per approved card");
      assert.equal(finalApproves[1].command, "approve_proposal");
      assert.equal(finalApproves[1].driverInstanceId, "inst-2", "the last approval targets the DB-Server instance");
      assert.equal(finalApproves[1].proposalId, "prop-2", "the last approval carries the DB-Server proposal id");
      assert.deepEqual(finalApproves[1].args, { command: "journalctl -u nginx" }, "the last approval carries the DB-Server draft args");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
    await new Promise((resolve) => staticServer.server.close(resolve));
  }
});
