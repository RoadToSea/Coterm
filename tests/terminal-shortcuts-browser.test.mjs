/** Actual plugin bundles and xterm keyboard/paste events; clipboard writes are isolated. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

for (const protocol of ["serial", "ssh"]) {
  test(`${protocol} Shell handles local shortcuts without sending them to the remote terminal`, async (t) => {
    const browserPath = findBrowser();
    if (!browserPath) { t.skip("No Chrome/Edge binary; terminal shortcuts were not browser-verified"); return; }
    const artifact = new URL(`../plugins/plugin-${protocol}/dist/frontend/index.js`, import.meta.url);
    const built = fs.statSync(artifact).mtimeMs;
    for (const source of [
      new URL(`../plugins/plugin-${protocol}/frontend/src/index.ts`, import.meta.url),
      new URL(`../plugins/plugin-${protocol}/frontend/src/i18n.ts`, import.meta.url),
      new URL("../packages/plugin-sdk/src/components/terminal-shortcuts.ts", import.meta.url),
      new URL("../packages/plugin-sdk/src/components/terminal-search.ts", import.meta.url),
    ]) assert.ok(built >= fs.statSync(source).mtimeMs, `rebuild plugin:${protocol}:build before testing`);
    const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
    const session = await openSession({ browserPath, totalTimeoutMs:30_000 });
    const { client } = session;
    const run = (source) => evaluate(client, source);
    const key = async (key, code, windowsVirtualKeyCode, modifiers = 0) => {
      const params = {key,code,windowsVirtualKeyCode,modifiers};
      await client.send("Input.dispatchKeyEvent", {type:"keyDown",...params});
      await client.send("Input.dispatchKeyEvent", {type:"keyUp",...params});
    };
    try {
      await client.send("Runtime.enable");
      await run(`(async () => {
        document.body.style.cssText = 'margin:0;width:1100px;height:800px';
        window.calls = []; window.copies = []; window.failCopy = false; window.locale = 'en';
        window.protocol = ${JSON.stringify(protocol)};
        // Never read or overwrite the user's system clipboard during this test.
        Object.defineProperty(navigator, 'clipboard', {configurable:true,value:{async writeText(text) {
          if (window.failCopy) throw new Error('clipboard unavailable');
          window.copies.push(text);
        }}});
        document.execCommand = () => false;
        const receipt = (output) => ({operationId:'test',status:'completed',output,error:null});
        const api = {
          i18n:{get locale() { return window.locale; },subscribe(fn) { window.translate = fn; return () => {}; }},
          async invoke(operation,args) {
            window.calls.push({operation,args});
            if (operation.endsWith('.status')) return receipt({
              connected:true, connectionId:'conn', config:{port:'COM1',baudRate:115200,dtr:true,rts:true,
                username:'user',host:'localhost'},signals:{dtr:true,rts:true},rxBytes:0,txBytes:0,commandQueue:0,
            });
            if (operation === 'serial.read') return receipt({text:args.cursor === 0 ? ${JSON.stringify("selected terminal output 中文\r\n")} : '',bytes:33,
              cursor:1,gap:false,closed:false,more:false,connectionId:'conn'});
            if (operation.endsWith('.read')) return receipt({data:args.cursor === (protocol === 'serial' ? 0 : '0')
              ? ${JSON.stringify(Buffer.from("selected terminal output 中文\r\n").toString("base64"))} : '',
              cursor:protocol === 'serial' ? 1 : '1',gap:false,closed:false,more:false,connectionId:'conn'});
            if (operation === 'serial.write') return receipt({bytes:new TextEncoder().encode(args.text).length});
            if (operation === 'ssh.input') return receipt({bytes:atob(args.data).length});
            if (operation === 'ssh.resize') return receipt({});
            throw new Error(operation);
          },
          async subscribeEvents(fn) { window.event = fn; return () => { window.unsubscribed = true; }; },
        };
        const module = await import(${JSON.stringify(moduleURL)});
        window.frontend = module.mount(document.body,api);
        window.root = document.body.firstElementChild.shadowRoot;
        window.raw = () => window.calls.filter(c => c.operation === 'serial.write' || c.operation === 'ssh.input')
          .map(c => c.operation === 'serial.write' ? c.args.text : atob(c.args.data)).join('');
        window.paste = (text) => {
          const clipboardData = new DataTransfer(); clipboardData.setData('text/plain',text);
          root.querySelector('.xterm-helper-textarea').dispatchEvent(new ClipboardEvent('paste',
            {clipboardData,bubbles:true,cancelable:true}));
        };
        window.output = async (text) => {
          await window.event({name:protocol === 'serial' ? 'serial.output' : 'ssh.terminal.output',
            payload:{connectionId:'conn',cursor:protocol === 'serial' ? ++window.cursor : String(++window.cursor),...(protocol === 'serial' ? {text,bytes:new TextEncoder().encode(text).length} : {data:btoa(text)})}});
        };
        window.cursor = 1;
      })()`);
      await waitForValue(client, `root.querySelector('.xterm-screen').textContent.includes('selected terminal output')`);
      await run(`root.querySelector('.xterm-helper-textarea').focus()`);

      await key("A", "KeyA", 65, 10); // Ctrl+Shift+A
      for (const [name, code, vk, modifiers] of [["c","KeyC",67,2],["C","KeyC",67,10],["Insert","Insert",45,2]]) {
        await key(name,code,vk,modifiers);
      }
      await waitForValue(client, `window.copies.length === 3`);
      for (const copy of await run(`window.copies`)) assert.match(copy, /selected terminal output 中文/);
      assert.equal(await run(`window.raw()`), "", "all three copy shortcuts are local");
      await key("Escape", "Escape", 27);
      assert.equal(await run(`window.raw()`), "", "Escape clears the terminal selection locally");
      await key("c", "KeyC", 67, 2);
      await waitForValue(client, `window.raw() === ${JSON.stringify("\u0003")}`);

      for (const [letter, vk] of [["a",65],["e",69],["r",82],["d",68],["l",76],["u",85],["w",87],["z",90]])
        await key(letter,`Key${letter.toUpperCase()}`,vk,2);
      const controls = "\u0003\u0001\u0005\u0012\u0004\u000c\u0015\u0017\u001a";
      await waitForValue(client, `window.raw() === ${JSON.stringify(controls)}`);

      // Synthetic shortcut events avoid reading the OS clipboard; real xterm paste
      // events below verify single delivery and newline/bracketed-paste handling.
      for (const flags of [
        {key:"v",code:"KeyV",ctrlKey:true},
        {key:"V",code:"KeyV",ctrlKey:true,shiftKey:true},
        {key:"Insert",code:"Insert",shiftKey:true},
      ]) {
        assert.equal(await run(`(() => {
          const area = root.querySelector('.xterm-helper-textarea');
          const event = new KeyboardEvent('keydown',{bubbles:true,cancelable:true,...${JSON.stringify(flags)}});
          area.dispatchEvent(event);
          area.dispatchEvent(new KeyboardEvent('keyup',{bubbles:true,...${JSON.stringify(flags)}}));
          return event.defaultPrevented;
        })()`), false, "paste remains a native browser action");
        await run(`window.paste(${JSON.stringify("paste\ntext")})`);
      }
      const pasted = controls + "paste\rtext".repeat(3);
      await waitForValue(client, `window.raw() === ${JSON.stringify(pasted)}`);
      await run(`window.output(${JSON.stringify("\u001b[?2004h")})`);
      await run(`window.paste(${JSON.stringify("safe\npaste")})`);
      const bracketed = pasted + "\u001b[200~safe\rpaste\u001b[201~";
      await waitForValue(client, `window.raw() === ${JSON.stringify(bracketed)}`);

      await t.test("Ctrl+F searches terminal output locally, navigates, updates, and closes", async () => {
        await run(`window.output(${JSON.stringify("needle first\r\nneedle second\r\nNEEDLE third\r\n")})`);
        await key("f", "KeyF", 70, 2);
        await waitForValue(client, `!root.querySelector('.terminal-find').hidden && root.activeElement === root.querySelector('.terminal-find input')`);
        await client.send("Input.insertText", {text:"needle"});
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent === '1 of 3'`);
        await key("Enter", "Enter", 13);
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent === '2 of 3'`);
        await key("Enter", "Enter", 13, 8);
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent === '1 of 3'`);
        await run(`root.querySelector('.terminal-find button').click()`);
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent === '3 of 3'`);
        await run(`root.querySelectorAll('.terminal-find button')[1].click()`);
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent === '1 of 3'`);
        await run(`window.output(${JSON.stringify("needle later\r\n")})`);
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent.endsWith('of 4')`);

        await run(`root.querySelector('.terminal-find input').focus()`);
        await key("f", "KeyF", 70, 2);
        await client.send("Input.insertText", {text:".*"});
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent === 'No matches'`);
        assert.equal(await run(`root.querySelector('.terminal-find button').disabled`), true, "search is literal, not an unbounded regular expression");
        await run(`window.locale = 'zh-CN'; window.translate()`);
        assert.equal(await run(`root.querySelector('.terminal-find input').getAttribute('aria-label')`), "在终端中查找");
        assert.equal(await run(`root.querySelector('.terminal-find-result').textContent`), "未找到匹配项");
        await key("f", "KeyF", 70, 2);
        await client.send("Input.insertText", {text:"中文"});
        await waitForValue(client, `root.querySelector('.terminal-find-result').textContent === '第 1 / 1 项'`);
        await key("Escape", "Escape", 27);
        await waitForValue(client, `root.querySelector('.terminal-find').hidden && root.activeElement?.classList.contains('xterm-helper-textarea')`);
        assert.equal(await run(`window.raw()`), bracketed, "search text and navigation never reach the remote shell");

        await key("f", "KeyF", 70, 2);
        assert.equal(await run(`root.querySelector('.terminal-find input').value`), "中文", "the query survives closing within this mounted view");
        assert.equal(await run(`root.querySelector('.terminal-find input').maxLength`), 512);
        if (protocol === "serial") {
          await run(`root.querySelectorAll('.modes button')[1].click()`);
          assert.equal(await run(`root.querySelector('.terminal-find').hidden`), true, "UART mode hides Shell search");
          await run(`root.querySelectorAll('.modes button')[0].click()`);
        } else {
          await run(`root.querySelectorAll('.terminal-find button')[2].click()`);
        }
        await waitForValue(client, `root.activeElement?.classList.contains('xterm-helper-textarea')`);
        await run(`window.locale = 'en'; window.translate()`);
      });

      // Shortcut handlers are scoped to the terminal, leaving ordinary inputs alone.
      await run(`(() => {
        const outside = document.createElement('input'); outside.value = 'outside editor';
        document.body.append(outside); outside.focus();
        outside.dispatchEvent(new KeyboardEvent('keydown',{key:'c',code:'KeyC',ctrlKey:true,bubbles:true}));
        outside.dispatchEvent(new KeyboardEvent('keydown',{key:'v',code:'KeyV',ctrlKey:true,bubbles:true}));
      })()`);
      await key("a", "KeyA", 65, 2);
      assert.equal(await run(`document.activeElement.selectionEnd - document.activeElement.selectionStart`), 14);
      assert.equal(await run(`window.raw()`), bracketed);
      assert.equal(await run(`window.copies.length`), 3);

      await run(`root.querySelector('.xterm-helper-textarea').focus(); window.failCopy = true; window.locale = 'zh-CN'; window.translate()`);
      await key("A", "KeyA", 65, 10);
      await key("c", "KeyC", 67, 2);
      await waitForValue(client, `root.querySelector('.notice').textContent === '无法复制终端选区，请重试复制。'`);
      assert.equal(await run(`window.raw()`), bracketed, "clipboard failure never sends an interrupt");
      await run(`window.frontend.unmount()`);
      assert.equal(await run(`window.unsubscribed`), true);
    } catch (error) {
      t.diagnostic(JSON.stringify(await run(`({calls:window.calls?.map(c => c.operation),
        notice:window.root?.querySelector('.notice')?.textContent,copies:window.copies?.length})`).catch(() => null)));
      throw error;
    } finally { await session.close(); }
  });
}
