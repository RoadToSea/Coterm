/** 真实串口前端 + 无头浏览器 + 插件 API 替身；不访问物理设备。 */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("human Shell uses terminal keys, UART uses its sender, and MCP queues stay separate", async (t) => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; serial terminal UI was not verified"); return; }
  const artifact = new URL("../plugins/plugin-serial/dist/frontend/index.js", import.meta.url);
  const built = fs.statSync(artifact).mtimeMs;
  assert.ok(built >= fs.statSync(new URL("../packages/plugin-sdk/src/components/terminal-shortcuts.ts", import.meta.url)).mtimeMs,
    "rebuild the serial frontend after changing shared terminal shortcuts");
  assert.ok(built >= fs.statSync(new URL("../packages/plugin-sdk/src/components/terminal-search.ts", import.meta.url)).mtimeMs,
    "rebuild the serial frontend after changing terminal search");
  assert.ok(built >= fs.statSync(new URL("../packages/plugin-sdk/src/components/toasts.ts", import.meta.url)).mtimeMs,
    "rebuild the serial frontend after changing shared notices");
  for (const file of ["index.ts", "i18n.ts", "console-settings.ts", "format-switch.ts", "toolbar.ts", "receive-newlines.ts"]) {
    assert.ok(built >= fs.statSync(new URL(`../plugins/plugin-serial/frontend/src/${file}`, import.meta.url)).mtimeMs,
      "run pnpm --filter coterm-plugin-serial-ui build before the serial browser test");
  }
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({ browserPath, totalTimeoutMs: 60_000 });
  const { client } = session;
  const run = (source) => evaluate(client, source);
  const key = async (key, code, windowsVirtualKeyCode, extra = {}) => {
    const params = { key, code, windowsVirtualKeyCode, ...extra };
    await client.send("Input.dispatchKeyEvent", { type: "keyDown", ...params });
    await client.send("Input.dispatchKeyEvent", { type: "keyUp", ...params });
  };
  const click = async selector => {
    const { x, y } = await run(`(() => { const r=root.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return {x:r.left+r.width/2,y:r.top+r.height/2}; })()`);
    await client.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
    await client.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
  };
  const dragToolbar = async (from, to, after, cancel = false) => {
    const start = await run(`(() => {
      const a=root.querySelector('[data-control=${from}]').getBoundingClientRect();
      return {x:a.left+a.width/2,y:a.top+a.height/2};
    })()`);
    await client.send("Input.setInterceptDrags", { enabled: true });
    try {
      const intercepted = client.waitFor("Input.dragIntercepted", 5000);
      await client.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...start });
      await client.send("Input.dispatchMouseEvent", { type: "mousePressed", ...start, button: "left", clickCount: 1 });
      await client.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: start.x + 12, y: start.y, button: "left", buttons: 1 });
      const { data } = await intercepted;
      assert.equal(data.items.some(item => item.mimeType === "text/plain"), false, 'dragging controls must not paste their IDs into text editors');
      assert.equal(await run(`root.querySelector('.toolbar-trash').hidden`), false, 'the trash appears only while dragging');
      assert.equal(await run(`getComputedStyle(root.querySelector('[data-control=${from}] > *')).animationPlayState`), 'paused');
      const end = await run(`(() => {
        const b=root.querySelector(${JSON.stringify(to === 'trash' ? '.toolbar-trash' : `[data-control=${to}]`)}).getBoundingClientRect();
        return {x:b.left+b.width*${to === 'trash' ? .5 : after ? .8 : .2},y:b.top+b.height/2};
      })()`);
      for (const type of ["dragEnter", "dragOver"]) await client.send("Input.dispatchDragEvent", { type, ...end, data });
      if (to === 'trash') assert.equal(await run(`root.querySelector('.toolbar-trash').dataset.over`), 'true');
      else assert.equal(await run(`root.querySelector('[data-control=${to}]').dataset.dropSide`), after ? 'after' : 'before');
      await client.send("Input.dispatchDragEvent", { type: cancel ? "dragCancel" : "drop", ...end, data });
      await client.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...end, button: "left", clickCount: 1 });
      assert.equal(await run(`root.querySelector('.toolbar-trash').hidden`), true, 'drop or cancel releases the trash target');
    } finally { await client.send("Input.setInterceptDrags", { enabled: false }); }
  };
  try {
    await client.send("Runtime.enable");
    await client.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await client.send("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
    await run(`(async () => {
      document.body.style.cssText = 'margin:0;width:1100px;height:800px';
      window.calls = []; window.locale = 'en'; window.connected = true;
      window.commandQueue = 0; window.txBytes = 0; window.cursor = 0;
      window.levels = {dtr:true,rts:true};
      window.config = {port:'COM1',baudRate:115200,dataBits:8,stopBits:1,parity:'none',dtr:true,rts:true};
      window.receipt = (output = null) => ({ operationId: 'human-input', status:'completed', output,
        error:null, proposalId:null, waitExpired:false });
      window.api = {
        pluginInstanceId: 'one',
        i18n: { get locale() { return window.locale; }, subscribe(fn) {
          window.translate = fn; return () => { window.localeUnsubscribed = true; };
        } },
        async invoke(operation, args) {
          window.calls.push({ operation, args });
          if (operation === 'serial.status') return window.receipt({
            config:window.config, connected:window.connected,
            connectionId:window.connected ? 'conn' : null, signals:window.levels,
            rxBytes:0, txBytes:window.txBytes, commandQueue:window.commandQueue,
          });
          if (operation === 'serial.read') return window.receipt({text:'',bytes:0,cursor:window.cursor,
            gap:false,closed:false,more:false,connectionId:'conn'});
          if (operation === 'serial.read_hex') return window.receipt({hex:window.hexOutput ?? '',bytes:(window.hexOutput ?? '').length/2,cursor:window.cursor,
            gap:false,closed:false,more:false,connectionId:'conn'});
          if (operation === 'serial.write' || operation === 'serial.write_hex') {
            if (window.holdWrite) await new Promise(resolve => { window.releaseWrite = resolve; });
            if (window.failWrite) throw new Error('simulated write failure');
            const bytes = operation === 'serial.write_hex' ? args.hex.length/2 : new TextEncoder().encode(args.text).length + ({none:0,cr:1,lf:1,crlf:2}[args.lineEnding]);
            window.txBytes += bytes;
            return window.receipt({bytes});
          }
          if (operation === 'serial.signals') { Object.assign(window.levels,args); return window.receipt(window.levels); }
          if (operation === 'serial.disconnect') { window.connected = false; return window.receipt({}); }
          if (operation === 'serial.connect') { window.connected = true; return window.receipt({}); }
          if (operation === 'serial.configure') { Object.assign(window.config, args); return window.receipt({}); }
          throw new Error('Unexpected human operation: ' + operation);
        },
        async subscribeEvents(fn) { window.event = fn; return () => { window.unsubscribed = true; }; },
      };
      const module = await import(${JSON.stringify(moduleURL)});
      window.frontend = module.mount(document.body, window.api);
      window.root = document.body.firstElementChild.shadowRoot;
      window.writes = () => window.calls.filter(c => c.operation === 'serial.write' || c.operation === 'serial.write_hex');
      window.sendUart = () => root.querySelector('.uart-send textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      window.raw = () => window.writes().map(c => c.args.text ?? '').join('');
    })()`);
    await waitForValue(client, `root.querySelector('.connect-button').dataset.connected === 'true' && root.activeElement?.classList.contains('xterm-helper-textarea')`);
    assert.equal(await run(`root.querySelector('.send') === null`), true, "Shell has no list editor or send toolbar");
    assert.equal(await run(`root.querySelector('.panel:not([hidden]) textarea:not(.xterm-helper-textarea)') === null`), true);
    assert.equal(await run(`root.querySelector('.uart').hidden`), true);

    // 通过浏览器真实输入事件验证 xterm → serial.write，不调用前端内部函数。
    await client.send("Input.insertText", { text: "pwd" });
    await key("Enter", "Enter", 13, { text: "\r" });
    await key("Tab", "Tab", 9);
    await key("ArrowUp", "ArrowUp", 38);
    await key("Backspace", "Backspace", 8);
    await key("c", "KeyC", 67, { modifiers: 2 });
    await waitForValue(client, `window.raw() === ${JSON.stringify("pwd\r\t\u001b[A\b\u0003")}`);
    assert.equal(await run(`window.writes().every(c => c.args.lineEnding === 'none')`), true,
      "terminal Enter and controls are raw bytes without an extra line ending");
    await client.send("Input.insertText", { text: "echo 中文" });
    await waitForValue(client, `window.raw().endsWith('echo 中文')`);
    assert.equal(await run(`window.calls.some(c => c.operation.endsWith('_hex'))`), false, "Shell never uses HEX interfaces");

    await run(`(async () => {
      window.cursor = 1;
      await window.event({name:'serial.output',payload:{connectionId:'conn',cursor:1,text:${JSON.stringify("device echo\r\n")},bytes:13}});
    })()`);
    await waitForValue(client, `root.querySelector('.xterm-screen').textContent.includes('device echo')`);

    // UART 发送框按原样发送文本或 HEX，不拆成命令列表。
    await run(`(() => {
      root.querySelectorAll('.modes button')[1].click();
      root.querySelector('.uart-send textarea').value = ${JSON.stringify("first\nsecond")};
      window.sendUart();
    })()`);
    await waitForValue(client, `window.writes().some(c => c.args.text === ${JSON.stringify("first\nsecond")})`);
    assert.deepEqual(await run(`window.writes().at(-1).args`),
      { text: "first\nsecond", lineEnding: "crlf" });
    await run(`root.querySelector('.history-toggle').click()`);
    assert.equal(await run(`root.querySelector('.send-history button').textContent`), "ASCII · first second");
    await run(`root.querySelector('.send-history button').click()`);
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), "first\nsecond");

    // 配置覆盖接收区，顶部/底部位置不变；折叠时不保留可聚焦控件。
    const receiveRect = await run(`JSON.stringify(root.querySelector('.uart-output').getBoundingClientRect().toJSON())`);
    const sendRect = await run(`JSON.stringify(root.querySelector('.uart-send').getBoundingClientRect().toJSON())`);
    await run(`root.querySelector('.settings-toggle').click()`);
    await waitForValue(client, `getComputedStyle(root.querySelector('.settings-panel')).transform === 'matrix(1, 0, 0, 1, 0, 0)'`);
    assert.equal(await run(`JSON.stringify(root.querySelector('.uart-output').getBoundingClientRect().toJSON())`), receiveRect);
    assert.equal(await run(`JSON.stringify(root.querySelector('.uart-send').getBoundingClientRect().toJSON())`), sendRect);
    assert.equal(await run(`root.querySelector('[name=baudRate]').readOnly`), true);
    assert.equal(await run(`root.querySelector('.settings-panel').inert`), false);
    const overlayBounds = await run(`(() => { const a=root.querySelector('.receive-area').getBoundingClientRect();
      const b=root.querySelector('.settings-panel').getBoundingClientRect(); return {top:b.top-a.top,bottom:b.bottom<=a.bottom+1}; })()`);
    assert.ok(Math.abs(overlayBounds.top) < 1 && overlayBounds.bottom);
    await key("Escape", "Escape", 27);
    assert.equal(await run(`root.querySelector('.settings-panel').inert`), true);
    assert.equal(await run(`root.activeElement.className`), "settings-toggle");

    // 快捷栏晃动调整、拖到垃圾桶移除与默认恢复只影响界面，不产生协议操作。
    assert.equal(await run(`root.querySelector('.connection-label,.state,.apply-settings,.receive-settings,.send-format-select,.uart-send .primary') === null`), true);
    assert.equal(await run(`root.querySelectorAll('.communication-settings fieldset, .communication-settings [name=port]').length`), 0);
    assert.equal(await run(`root.querySelector('.settings-toggle').nextElementSibling.classList.contains('toolbar-toggle')`), true);
    assert.equal(await run(`['pause','clear','save'].every(id=>root.querySelector('[data-control='+id+'] button[aria-label] svg'))`), true);
    await run(`root.querySelector('.settings-toggle').click(); root.querySelector('[name=baudRate]').click()`);
    await waitForValue(client, `root.querySelector('.coterm-toast[data-tone=warning]')`);
    assert.equal(await run(`root.querySelector('[name=baudRate]').value`), '115200');
    assert.equal(await run(`root.querySelector('[role=dialog],dialog') === null`), true);
    assert.equal(await run(`JSON.stringify(root.querySelector('.uart-send').getBoundingClientRect().toJSON())`), sendRect);
    await run(`root.querySelector('[name=dtr]').click()`);
    await waitForValue(client, `window.levels.dtr===false && !root.querySelector('[name=dtr]').disabled`);
    assert.equal(await run(`window.calls.at(-1).operation`), 'serial.signals');
    await run(`root.querySelector('.toolbar-toggle').click()`);
    assert.equal(await run(`root.querySelector('.settings-panel').inert`), true);
    assert.equal(await run(`root.querySelector('.toolbar-customization').hidden`), false);
    assert.equal(await run(`root.querySelectorAll('.toolbar-customization input,.toolbar-settings-list,.toolbar-setting').length`), 0, 'the checkbox list is removed');
    assert.equal(await run(`root.querySelector('.toolbar-trash').hidden`), true);
    assert.equal(await run(`getComputedStyle(root.querySelector('.export-receive')).animationName`), 'serial-toolbar-wiggle');
    await run(`window.firstToolbarTransform=getComputedStyle(root.querySelector('.export-receive')).transform`);
    await waitForValue(client, `getComputedStyle(root.querySelector('.export-receive')).transform !== window.firstToolbarTransform`);
    await client.send("Emulation.setEmulatedMedia", { features: [{name: "prefers-reduced-motion", value: "reduce"}] });
    assert.equal(await run(`getComputedStyle(root.querySelector('.export-receive')).animationName`), 'none');
    await client.send("Emulation.setEmulatedMedia", { features: [{name: "prefers-reduced-motion", value: "no-preference"}] });
    assert.equal(await run(`root.querySelector('.toolbar-customization').getBoundingClientRect().top >= root.querySelector('.uart-bar').getBoundingClientRect().bottom - 1`), true, 'settings leave the actual toolbar exposed for dragging');
    const toolbarCallCount = await run(`window.calls.length`);
    await dragToolbar('timestamps', 'trash', false, true);
    assert.equal(await run(`root.querySelector('[data-control=timestamps]').hidden`), false, 'cancelling over the trash does not remove a control');
    await dragToolbar('timestamps', 'trash', false);
    await dragToolbar('save', 'clear', false);
    assert.equal(await run(`root.querySelector('[data-control=timestamps]').hidden`), true);
    assert.deepEqual(await run(`[...root.querySelector('.uart-bar').children].map(el=>el.dataset.control)`),
      ['format','timestamps','autoscroll','pause','save','clear','stats']);
    assert.equal(await run(`root.querySelector('.toolbar-customization').hidden`), false, 'dragging toolbar controls keeps customization active');
    assert.equal(await run(`root.querySelectorAll('[data-dragging],[data-drop-side]').length`), 0);
    await dragToolbar('save', 'format', false, true);
    assert.deepEqual(await run(`[...root.querySelector('.uart-bar').children].map(el=>el.dataset.control)`),
      ['format','timestamps','autoscroll','pause','save','clear','stats'], 'cancelled drags leave the order unchanged');
    await dragToolbar('save', 'clear', true);
    assert.deepEqual(await run(`[...root.querySelector('.uart-bar').children].map(el=>el.dataset.control)`),
      ['format','timestamps','autoscroll','pause','clear','save','stats'], 'controls can also be dragged to the right');
    await run(`root.querySelector('[data-control=save]').focus()`);
    await key("ArrowLeft", "ArrowLeft", 37, { modifiers: 1 });
    assert.deepEqual(await run(`[...root.querySelector('.uart-bar').children].map(el=>el.dataset.control)`),
      ['format','timestamps','autoscroll','pause','save','clear','stats'], 'Alt + arrows offer keyboard reordering');
    assert.equal(await run(`window.calls.length`), toolbarCallCount);
    await run(`root.querySelector('[data-restore=timestamps]').click()`);
    assert.equal(await run(`root.querySelector('[data-control=timestamps]').hidden`), false);
    await run(`root.querySelector('[data-control=save]').focus()`);
    await key("Delete", "Delete", 46);
    assert.equal(await run(`root.querySelector('[data-control=save]').hidden`), true, 'keyboard users can remove the focused control');
    await run(`root.querySelector('[data-restore=save]').click()`);
    assert.equal(await run(`root.querySelector('[data-control=save]').hidden`), false);
    await run(`for(const slot of [...root.querySelector('.uart-bar').children]) {
      slot.focus(); slot.dispatchEvent(new KeyboardEvent('keydown',{key:'Delete',bubbles:true,cancelable:true}));
    }`);
    assert.equal(await run(`root.querySelectorAll('.toolbar-item:not([hidden])').length`), 0);
    assert.equal(await run(`root.querySelectorAll('.toolbar-removed button').length`), 7, 'removed controls can always be restored');
    assert.equal(await run(`root.activeElement.className`), 'toolbar-done');
    await run(`root.querySelector('.toolbar-reset').click(); root.querySelector('.toolbar-done').click()`);
    assert.deepEqual(await run(`[...root.querySelector('.uart-bar').children].map(el=>el.dataset.control)`),
      ['format','timestamps','autoscroll','pause','clear','save','stats']);
    assert.equal(await run(`root.querySelector('[data-control=save]').draggable || root.querySelector('.export-receive').inert`), false, 'leaving customization restores normal button actions');
    assert.equal(await run(`getComputedStyle(root.querySelector('.export-receive')).animationName`), 'none');
    assert.equal(await run(`getComputedStyle(root.querySelector('.export-receive')).willChange`), 'auto');
    assert.equal(await run(`window.calls.length`), toolbarCallCount, 'removing and restoring controls never sends device operations');
    assert.equal(await run(`root.querySelector('.history-toggle').getAttribute('aria-label')`), 'Send history');

    // 滑块使用 UTF-8 转换草稿，字节以空格分隔；非法 HEX 只提示不发送。
    await run(`root.querySelector('.uart-send textarea').value='A中'`);
    await click('.send-mode button[data-value=hex]');
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), '41 e4 b8 ad');
    await click('.send-mode button[data-value=text]');
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), 'A中');
    const slider = await run(`(() => { const r=root.querySelector('.send-mode').getBoundingClientRect(); return {x:r.left+20,y:r.top+r.height/2}; })()`);
    await client.send("Input.dispatchMouseEvent", { type: "mousePressed", ...slider, button: "left", clickCount: 1 });
    await client.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: slider.x + 36, y: slider.y, button: "left", buttons: 1 });
    await client.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: slider.x + 65, y: slider.y, button: "left", clickCount: 1 });
    assert.equal(await run(`root.querySelector('.send-mode').dataset.value`), 'hex', 'pointer swipe changes format and converts the draft');
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), '41 e4 b8 ad');
    await run(`root.querySelector('.uart-send textarea').value='zz'; window.sendUart()`);
    await waitForValue(client, `root.querySelector('.coterm-toast[data-tone=warning]')?.textContent.includes('hexadecimal')`);
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), 'zz');

    await run(`(() => {
      root.querySelector('.send-mode button[data-value=hex]').click();
      root.querySelector('.uart-send textarea').value = '00 80 FF';
      root.querySelector('.uart-send textarea').dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',bubbles:true}));
    })()`);
    await waitForValue(client, `window.writes().at(-1)?.args.hex === '0080ff'`);
    assert.deepEqual(await run(`window.writes().at(-1)`), { operation:"serial.write_hex", args:{hex:"0080ff"} });
    await run(`window.hexOutput = '0080ff'; root.querySelector('.uart-bar .format-switch button').focus()`);
    await key("ArrowRight", "ArrowRight", 39);
    await waitForValue(client, `window.calls.some(c => c.operation === 'serial.read_hex')`);
    await waitForValue(client, `root.querySelector('.uart-output').textContent.includes('00 80 ff')`);
    await key("ArrowLeft", "ArrowLeft", 37);
    assert.equal(await run(`root.querySelector('.uart-bar .format-switch').dataset.value`), 'text');
    const count = await run(`window.writes().length`);
    assert.equal(await run(`(() => {
      const event = new KeyboardEvent('keydown', {key:'Enter',shiftKey:true,bubbles:true,cancelable:true});
      root.querySelector('.uart-send textarea').dispatchEvent(event);
      return event.defaultPrevented;
    })()`), false, "Shift+Enter remains available for editing multiline UART text");
    assert.equal(await run(`window.writes().length`), count);
    for (const extra of ["isComposing:true", "keyCode:229", "repeat:true"]) {
      await run(`root.querySelector('.uart-send textarea').dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',${extra},bubbles:true,cancelable:true}))`);
      assert.equal(await run(`window.writes().length`), count, "IME confirmation and held Enter never send");
    }

    // 外部 MCP 操作仍可更新计数；队列在途时人工输入不混写。
    await run(`(async () => {
      window.commandQueue = 1;
      await window.event({name:'serial.status',payload:{}});
      await window.event({name:'serial.command',payload:{connectionId:'conn',txBytes:99,finished:false}});
      window.sendUart();
    })()`);
    await waitForValue(client, `root.querySelector('.coterm-toasts').textContent.includes('MCP command queue is busy')`);
    assert.match(await run(`root.querySelector('.stats').textContent`), /TX 99 B/);
    await run(`root.querySelectorAll('.modes button')[0].click()`);
    await waitForValue(client, `root.activeElement?.classList.contains('xterm-helper-textarea')`);
    await client.send("Input.insertText", { text: "blocked" });
    assert.equal(await run(`window.writes().length`), count, "MCP queue blocks both human send paths");

    await run(`(async () => {
      window.commandQueue = 0; window.txBytes = 99;
      await window.event({name:'serial.command',payload:{connectionId:'conn',txBytes:99,finished:true}});
    })()`);
    await client.send("Input.insertText", { text: "ok" });
    await waitForValue(client, `window.writes().length > ${count}`);
    assert.equal(await run(`window.writes().at(-1).args.text`), "ok", "human input resumes after MCP finishes");

    await run(`window.locale = 'zh-CN'; window.translate()`);
    assert.equal(await run(`root.querySelector('.modes button').textContent`), "终端");
    assert.equal(await run(`root.querySelector('.repeat-button').textContent`), "定时发送");
    assert.equal(await run(`root.querySelector('.uart-output').textContent.includes('device echo')`), true,
      "mode and language changes keep captured output");
    assert.equal(await run(`window.calls.some(c => c.operation === 'serial.command' || c.operation === 'serial.connect')`), false,
      "human UI never submits command lists or reconnects when changing mode or locale");

    await run(`root.querySelector('.connect-button').click()`);
    await waitForValue(client, `root.querySelector('.connect-button').dataset.connected === 'false'`);
    const disconnectedCount = await run(`window.writes().length`);
    await run(`root.querySelector('.xterm-helper-textarea').focus()`);
    await client.send("Input.insertText", { text: "disconnected" });
    assert.equal(await run(`window.writes().length`), disconnectedCount);

    // 离线草稿只在用户点击连接时应用；后台快照不能覆盖未保存的草稿。
    const beforeConfigure = await run(`window.calls.filter(c=>c.operation==='serial.configure').length`);
    await run(`root.querySelector('.settings-toggle').click();
      root.querySelector('[name=baudRate]').value='9600';
      root.querySelector('[name=baudRate]').dispatchEvent(new Event('input'))`);
    assert.equal(await run(`window.config.baudRate`), 115200);
    await run(`window.event({name:'serial.status',payload:{}})`);
    assert.equal(await run(`root.querySelector('[name=baudRate]').value`), '9600');
    assert.equal(await run(`window.calls.filter(c=>c.operation==='serial.configure').length`), beforeConfigure);
    assert.equal(await run(`window.writes().length`), disconnectedCount);
    await run(`root.querySelector('[name=baudRate]').value='0'; root.querySelector('.connect-button').click()`);
    await waitForValue(client, `root.querySelector('.coterm-toast[data-tone=warning]')?.textContent.includes('通信参数')`);
    assert.equal(await run(`window.connected`), false);
    assert.equal(await run(`window.calls.filter(c=>c.operation==='serial.configure').length`), beforeConfigure);
    await run(`root.querySelector('[name=baudRate]').value='9600';
      root.querySelector('.settings-toggle').click(); root.querySelector('.connect-button').click()`);
    await waitForValue(client, `root.querySelector('.connect-button').dataset.connected === 'true'`);
    assert.equal(await run(`window.config.baudRate`), 9600);
    assert.deepEqual(await run(`window.calls.filter(c=>['serial.configure','serial.connect'].includes(c.operation)).map(c=>c.operation)`),
      ['serial.configure','serial.connect']);
    await run(`root.querySelectorAll('.modes button')[1].click()`);

    // 文件导入只填输入框，HEX 保留原始字节，错误不覆盖已有草稿。
    const importFile = (bytes, name) => run(`(() => {
      const transfer = new DataTransfer(); transfer.items.add(new File([new Uint8Array(${JSON.stringify(bytes)})], ${JSON.stringify(name)}));
      const input = root.querySelector('input[type=file]'); input.files = transfer.files;
      input.dispatchEvent(new Event('change'));
    })()`);
    await importFile([0, 128, 255], "bytes.bin");
    await waitForValue(client, `root.querySelector('.uart-send textarea').value === '00 80 ff'`);
    assert.equal(await run(`window.writes().length`), disconnectedCount);
    await run(`root.querySelector('.send-mode button[data-value=text]').click()`);
    assert.equal(await run(`root.querySelector('.send-mode').dataset.value`), 'hex', 'binary bytes cannot be silently decoded as UTF-8');
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), '00 80 ff');
    await run(`root.querySelector('.uart-send textarea').value=''; root.querySelector('.send-mode button[data-value=text]').click()`);
    await importFile([104, 101, 108, 108, 111], "hello.txt");
    await waitForValue(client, `root.querySelector('.uart-send textarea').value === 'hello'`);
    await importFile([255], "not-text.bin");
    await waitForValue(client, `root.querySelector('.coterm-toasts').textContent.includes('UTF-8')`);
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), "hello");
    await importFile(new Array(16385).fill(65), "large.txt");
    await waitForValue(client, `root.querySelector('.coterm-toasts').textContent.includes('16 KiB')`);
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), "hello");

    // 真实 Shift+Enter 编辑；发送保留输入中的换行。
    await run(`root.querySelector('.uart-send textarea').focus(); root.querySelector('.uart-send textarea').setSelectionRange(5,5)`);
    await key("Enter", "Enter", 13, { modifiers: 8, text: "\r" });
    await client.send("Input.insertText", { text: "world" });
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), "hello\nworld");
    await key("Enter", "Enter", 13, { text: "\r" });
    await waitForValue(client, `window.writes().at(-1).args.text === 'hello\\nworld'`);

    // 保存的是捕获缓冲，而非显示中经格式化或暂停的 DOM 文本；URL 在卸载时释放。
    await run(`(() => {
      window.savedBlobs=[]; window.revoked=[];
      URL.createObjectURL=blob => { window.savedBlobs.push(blob); return 'blob:test-'+window.savedBlobs.length; };
      URL.revokeObjectURL=url => window.revoked.push(url);
      HTMLAnchorElement.prototype.click=function() { window.downloadName=this.download; };
      root.querySelector('.receive-filename').value='../receive.txt';
      root.querySelector('.export-receive').click();
    })()`);
    // Reconnect cleared the old receive log; put new data through the actual event path.
    await run(`(async () => { window.cursor++;
      await window.event({name:'serial.output',payload:{connectionId:'conn',cursor:window.cursor,text:'saved\\r\\n',bytes:7}});
      root.querySelector('.export-receive').click(); })()`);
    await waitForValue(client, `window.savedBlobs.length === 1`);
    assert.equal(await run(`window.savedBlobs[0].text()`), "saved\r\n");
    assert.equal(await run(`window.downloadName`), "_receive.txt");

    // 大小校验包含 UTF-8 和结束符，拒绝后不写入、不记录为发送成功。
    const beforeOversized = await run(`window.writes().length`);
    await run(`root.querySelector('.uart-send textarea').value='中'.repeat(6000); window.sendUart()`);
    await waitForValue(client, `root.querySelector('.coterm-toasts').textContent.includes('16384')`);
    assert.equal(await run(`window.writes().length`), beforeOversized);
    await run(`root.querySelector('.uart-send textarea').value='repeat'`);

    // 慢写入只允许一次在途；停止时不追发积压，失败不自动重试。
    await run(`window.holdWrite = true; root.querySelector('.uart-send input[type=number]').value='20';
      root.querySelector('.repeat-button').click()`);
    await waitForValue(client, `typeof window.releaseWrite === 'function'`);
    const heldCount = await run(`window.writes().length`);
    await run(`new Promise(resolve => setTimeout(resolve, 100))`);
    assert.equal(await run(`window.writes().length`), heldCount);
    await run(`root.querySelector('.repeat-button').click(); window.holdWrite=false; window.releaseWrite()`);
    await run(`new Promise(resolve => setTimeout(resolve, 80))`);
    assert.equal(await run(`window.writes().length`), heldCount);
    await run(`window.failWrite=true; root.querySelector('.repeat-button').click()`);
    await waitForValue(client, `root.querySelector('.coterm-toasts').textContent.includes('simulated write failure')`);
    const failedCount = await run(`window.writes().length`);
    await run(`new Promise(resolve => setTimeout(resolve, 80))`);
    assert.equal(await run(`window.writes().length`), failedCount);
    assert.equal(await run(`root.querySelector('.repeat-button').getAttribute('aria-pressed')`), "false");
    await run(`window.failWrite=false`);

    // 历史有界、去重；选取恢复当时的发送格式与行结束符。
    await run(`(async () => { for(let i=0;i<55;i++) {
      root.querySelector('.uart-send textarea').value='history-'+i;
      root.querySelector('.uart-send textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await new Promise(resolve=>setTimeout(resolve,0));
    } root.querySelector('.history-toggle').click(); })()`);
    assert.equal(await run(`root.querySelectorAll('.send-history button').length`), 50);
    assert.equal(await run(`root.querySelector('.send-history button').textContent`), "ASCII · history-54");
    await run(`root.querySelector('.send-history button').click(); window.sendUart()`);
    await run(`new Promise(resolve=>setTimeout(resolve,0))`);
    assert.equal(await run(`root.querySelectorAll('.send-history button').length`), 50);

    await run(`root.querySelector('.repeat-button').click()`);
    await waitForValue(client, `root.querySelector('.repeat-button').getAttribute('aria-pressed')==='true'`);
    await run(`(async () => { window.connected=false; await window.event({name:'serial.status',payload:{}}); })()`);
    assert.equal(await run(`root.querySelector('.repeat-button').getAttribute('aria-pressed')`), "false");
    const stoppedAtDisconnect = await run(`window.writes().length`);
    await run(`root.querySelector('.connect-button').click()`);
    await waitForValue(client, `root.querySelector('.connect-button').dataset.connected === 'true'`);
    await run(`new Promise(resolve=>setTimeout(resolve,80))`);
    assert.equal(await run(`window.writes().length`), stoppedAtDisconnect, "reconnect does not resume repeating");

    // 窄容器下发送控件换行，配置滚动限于接收区域。
    await run(`document.body.style.width='360px'; root.querySelector('.settings-toggle').click()`);
    await waitForValue(client, `getComputedStyle(root.querySelector('.settings-panel')).transform === 'matrix(1, 0, 0, 1, 0, 0)'`);
    assert.equal(await run(`root.querySelector('.layout').scrollWidth <= 360`), true);
    assert.equal(await run(`root.querySelector('.settings-panel').clientHeight <= root.querySelector('.receive-area').clientHeight`), true);
    await run(`document.body.style.width='1100px'; root.querySelector('.settings-toggle').click()`);

    await run(`root.querySelector('.repeat-button').click()`);
    const unmountCount = await run(`(() => { const count=window.writes().length; window.frontend.unmount(); return count; })()`);
    assert.equal(await run(`window.unsubscribed && window.localeUnsubscribed && document.body.childElementCount === 0`), true);
    assert.deepEqual(await run(`window.revoked`), ["blob:test-1"]);
    await run(`new Promise(resolve => setTimeout(resolve, 80))`);
    assert.equal(await run(`window.writes().length`), unmountCount, "unmount cancels the repeat timer");
  } catch (error) {
    t.diagnostic(JSON.stringify(await run(`({
      calls:window.calls?.slice(-6), state:root.querySelector('.connect-button')?.textContent,
      notice:root.querySelector('.coterm-toasts')?.textContent,
      buttons:[...root.querySelectorAll('.bar button')].map(b => ({text:b.textContent,disabled:b.disabled})),
    })`).catch(() => null)));
    throw error;
  } finally { await session.close(); }
});
