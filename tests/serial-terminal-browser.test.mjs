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
  for (const file of ["index.ts", "i18n.ts"]) {
    assert.ok(built >= fs.statSync(new URL(`../plugins/plugin-serial/frontend/src/${file}`, import.meta.url)).mtimeMs,
      "run pnpm --filter coterm-plugin-serial-ui build before the serial browser test");
  }
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({ browserPath, totalTimeoutMs: 30_000 });
  const { client } = session;
  const run = (source) => evaluate(client, source);
  const key = async (key, code, windowsVirtualKeyCode, extra = {}) => {
    const params = { key, code, windowsVirtualKeyCode, ...extra };
    await client.send("Input.dispatchKeyEvent", { type: "keyDown", ...params });
    await client.send("Input.dispatchKeyEvent", { type: "keyUp", ...params });
  };
  try {
    await client.send("Runtime.enable");
    await run(`(async () => {
      document.body.style.cssText = 'margin:0;width:1100px;height:800px';
      window.calls = []; window.locale = 'en'; window.connected = true;
      window.commandQueue = 0; window.txBytes = 0; window.cursor = '0';
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
            config:{port:'COM1',baudRate:115200,dtr:true,rts:true}, connected:window.connected,
            connectionId:window.connected ? 'conn' : null, signals:{dtr:true,rts:true},
            rxBytes:0, txBytes:window.txBytes, commandQueue:window.commandQueue,
          });
          if (operation === 'serial.read') return window.receipt({data:'',cursor:window.cursor,
            gap:false,closed:false,more:false,connectionId:'conn'});
          if (operation === 'serial.write') {
            const bytes = atob(args.data).length + ({none:0,cr:1,lf:1,crlf:2}[args.lineEnding]);
            window.txBytes += bytes;
            return window.receipt({bytes});
          }
          if (operation === 'serial.disconnect') { window.connected = false; return window.receipt({}); }
          throw new Error('Unexpected human operation: ' + operation);
        },
        async subscribeEvents(fn) { window.event = fn; return () => { window.unsubscribed = true; }; },
      };
      const module = await import(${JSON.stringify(moduleURL)});
      window.frontend = module.mount(document.body, window.api);
      window.root = document.body.firstElementChild.shadowRoot;
      window.writes = () => window.calls.filter(c => c.operation === 'serial.write');
      window.raw = () => window.writes().map(c => atob(c.args.data)).join('');
    })()`);
    await waitForValue(client, `root.querySelector('.state').textContent === 'Connected' && root.activeElement?.classList.contains('xterm-helper-textarea')`);
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
    await waitForValue(client, `window.raw() === ${JSON.stringify("pwd\r\t\u001b[A\u007f\u0003")}`);
    assert.equal(await run(`window.writes().every(c => c.args.lineEnding === 'none')`), true,
      "terminal Enter and controls are raw bytes without an extra line ending");
    await client.send("Input.insertText", { text: "echo 中文" });
    await waitForValue(client, `new TextDecoder().decode(Uint8Array.from(window.raw(), c => c.charCodeAt(0))).endsWith('echo 中文')`);

    await run(`(async () => {
      window.cursor = '1';
      await window.event({name:'serial.output',payload:{connectionId:'conn',cursor:'1',data:btoa(${JSON.stringify("device echo\r\n")})}});
    })()`);
    await waitForValue(client, `root.querySelector('.xterm-screen').textContent.includes('device echo')`);

    // UART 发送框按原样发送文本或 HEX，不拆成命令列表。
    await run(`(() => {
      root.querySelectorAll('.modes button')[1].click();
      root.querySelector('.uart-send textarea').value = ${JSON.stringify("first\nsecond")};
      root.querySelector('.uart-send .primary').click();
    })()`);
    await waitForValue(client, `window.writes().some(c => c.args.data === btoa(${JSON.stringify("first\nsecond")}))`);
    assert.deepEqual(await run(`window.writes().at(-1).args`),
      { data: Buffer.from("first\nsecond").toString("base64"), lineEnding: "crlf" });
    await run(`(() => {
      root.querySelector('.uart-send input[type=checkbox]').checked = true;
      root.querySelector('.uart-send select').value = 'none';
      root.querySelector('.uart-send textarea').value = '00 80 FF';
      root.querySelector('.uart-send textarea').dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',bubbles:true}));
    })()`);
    await waitForValue(client, `window.writes().at(-1)?.args.data === 'AID/'`);
    assert.equal(await run(`window.writes().at(-1).args.lineEnding`), "none");
    const count = await run(`window.writes().length`);
    assert.equal(await run(`(() => {
      const event = new KeyboardEvent('keydown', {key:'Enter',shiftKey:true,bubbles:true,cancelable:true});
      root.querySelector('.uart-send textarea').dispatchEvent(event);
      return event.defaultPrevented;
    })()`), false, "Shift+Enter remains available for editing multiline UART text");
    assert.equal(await run(`window.writes().length`), count);

    // 外部 MCP 操作仍可更新计数；队列在途时人工输入不混写。
    await run(`(async () => {
      window.commandQueue = 1;
      await window.event({name:'serial.status',payload:{}});
      await window.event({name:'serial.command',payload:{connectionId:'conn',txBytes:99,finished:false}});
      root.querySelector('.uart-send .primary').click();
    })()`);
    await waitForValue(client, `root.querySelector('.notice').textContent.includes('MCP command queue is busy')`);
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
    assert.equal(await run(`window.writes().at(-1).args.data`), "b2s=", "human input resumes after MCP finishes");

    await run(`window.locale = 'zh-CN'; window.translate()`);
    assert.equal(await run(`root.querySelector('.modes button').textContent`), "终端");
    assert.equal(await run(`root.querySelector('.uart-send .primary').textContent`), "发送");
    assert.equal(await run(`root.querySelector('.uart-output').textContent.includes('device echo')`), true,
      "mode and language changes keep captured output");
    assert.equal(await run(`window.calls.some(c => c.operation === 'serial.command' || c.operation === 'serial.connect')`), false,
      "human UI never submits command lists or reconnects when changing mode or locale");

    await run(`root.querySelectorAll('.bar button').item(root.querySelectorAll('.bar button').length - 1).click()`);
    await waitForValue(client, `root.querySelector('.state').textContent === '未连接'`);
    const disconnectedCount = await run(`window.writes().length`);
    await run(`root.querySelector('.xterm-helper-textarea').focus()`);
    await client.send("Input.insertText", { text: "disconnected" });
    assert.equal(await run(`window.writes().length`), disconnectedCount);

    await run(`window.frontend.unmount()`);
    assert.equal(await run(`window.unsubscribed && window.localeUnsubscribed && document.body.childElementCount === 0`), true);
  } catch (error) {
    t.diagnostic(JSON.stringify(await run(`({
      calls:window.calls?.slice(-6), state:root.querySelector('.state')?.textContent,
      notice:root.querySelector('.notice')?.textContent,
      buttons:[...root.querySelectorAll('.bar button')].map(b => ({text:b.textContent,disabled:b.disabled})),
    })`).catch(() => null)));
    throw error;
  } finally { await session.close(); }
});
