/** 真实串口前端 + 无头浏览器 + 插件 API 替身；不访问物理设备。 */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("serial Shell list submits once, shows progress, cancels, and preserves UART raw sending", async (t) => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; serial command UI was not verified"); return; }
  const artifact = new URL("../plugins/plugin-serial/dist/frontend/index.js", import.meta.url);
  const built = fs.statSync(artifact).mtimeMs;
  for (const file of ["index.ts", "i18n.ts", "command-list.ts"]) {
    assert.ok(built >= fs.statSync(new URL(`../plugins/plugin-serial/frontend/src/${file}`, import.meta.url)).mtimeMs,
      "run pnpm --filter coterm-plugin-serial-ui build before the serial browser test");
  }
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({ browserPath, totalTimeoutMs: 30_000 });
  const { client } = session;
  const run = (source) => evaluate(client, source);
  try {
    await client.send("Runtime.enable");
    await run(`(async () => {
      document.body.style.cssText = 'margin:0;width:1100px;height:800px';
      window.calls = []; window.record = null; window.locale = 'en';
      window.receipt = (status, output = null) => ({ operationId: 'list-one', status, output, error: null, proposalId: null, waitExpired: false });
      window.api = {
        pluginInstanceId: 'one',
        i18n: { get locale() { return window.locale; }, subscribe(fn) { window.translate = fn; return () => {}; } },
        async invoke(operation, args) {
          window.calls.push({ operation, args });
          if (operation === 'serial.status') return window.receipt('completed', {
            config: { port:'COM6', baudRate:115200, dtr:true, rts:true }, connected:true,
            connectionId:'conn', signals:{dtr:true,rts:true}, rxBytes:0, txBytes:0, commandQueue:0,
          });
          if (operation === 'serial.read') return window.receipt('completed', {data:'',cursor:'0',gap:false,closed:false,more:false,connectionId:'conn'});
          if (operation === 'serial.command') { window.record = window.receipt('running'); return window.record; }
          if (operation === 'serial.write') return window.receipt('completed', { bytes:3 });
          throw new Error(operation);
        },
        async getOperation() { return window.record; },
        async cancelOperation(id) { window.cancelled = id; window.record = window.receipt('cancelled'); return window.record; },
        async subscribeEvents(fn) { window.event = fn; return () => { window.unsubscribed = true; }; },
      };
      const module = await import(${JSON.stringify(moduleURL)});
      window.frontend = module.mount(document.body, window.api);
      window.root = document.body.firstElementChild.shadowRoot;
    })()`);
    await waitForValue(client, `window.root?.querySelector('.send button')?.disabled === false`);
    assert.equal(await run(`root.querySelector('.send label span').textContent`), "Commands (one per line)");
    await run(`(() => {
      const text = root.querySelector('.send textarea'); text.value = ${JSON.stringify("cd /tmp\npwd\n")};
      text.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',bubbles:true}));
    })()`);
    assert.equal(await run(`calls.filter(c => c.operation === 'serial.command').length`), 0, "Enter edits the list");
    await run(`(() => {
      const text = root.querySelector('.send textarea');
      text.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',ctrlKey:true,bubbles:true}));
      root.querySelector('.send button').click();
    })()`);
    await waitForValue(client, `calls.filter(c => c.operation === 'serial.command').length === 1 && !root.querySelectorAll('.send button')[1].disabled`);
    assert.deepEqual(await run(`calls.find(c => c.operation === 'serial.command').args`),
      { commands:["cd /tmp", "pwd"], lineEnding:"cr", intervalMs:100 });
    await run(`window.event({name:'serial.command',payload:{connectionId:'conn',operationId:'list-one',sent:1,total:2,txBytes:8,finished:false}})`);
    assert.match(await run(`root.querySelector('.notice').textContent`), /Sent 1\/2/);
    await run(`root.querySelectorAll('.send button')[1].click()`);
    await waitForValue(client, `root.querySelector('.notice').textContent.includes('Queue stopped') && !root.querySelector('.send button').disabled`);
    assert.equal(await run(`window.cancelled`), "list-one");
    await run(`window.locale = 'zh-CN'; window.translate()`);
    assert.equal(await run(`root.querySelector('.send button').textContent`), "发送列表");
    await run(`(() => { root.querySelectorAll('.modes button')[1].click(); const text = root.querySelector('.uart-send textarea');
      text.value = 'ok'; root.querySelector('.uart-send .primary').click(); })()`);
    await waitForValue(client, `calls.some(c => c.operation === 'serial.write')`);
    assert.equal(await run(`calls.find(c => c.operation === 'serial.write').args.data`), "b2s=");
    await run(`window.frontend.unmount()`);
    assert.equal(await run(`window.unsubscribed && document.body.childElementCount === 0`), true);
  } finally { await session.close(); }
});
