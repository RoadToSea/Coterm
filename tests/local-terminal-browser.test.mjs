import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("Local starts explicitly on first mount and preserves running or exited shells on remount", async (t) => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; Local terminal UI was not verified"); return; }
  const artifact = new URL("../drivers/driver-local/dist/frontend/index.js", import.meta.url);
  assert.ok(fs.statSync(artifact).mtimeMs >= fs.statSync(new URL("../drivers/driver-local/frontend/src/index.ts", import.meta.url)).mtimeMs,
    "rebuild the Local frontend before this test");
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({ browserPath, totalTimeoutMs: 30_000 });
  const run = (source) => evaluate(session.client, source);
  try {
    await session.client.send("Runtime.enable");
    await run(`(async () => {
      document.body.style.cssText = 'margin:0;width:1000px;height:700px';
      window.container = document.createElement('div'); container.style.height = '700px'; document.body.append(container);
      window.frontend = await import(${JSON.stringify(moduleURL)});
      window.calls = []; window.unsubscribed = 0;
      window.localStatus = { shellId:null, shell:'test-shell', family:'cmd', cwd:'.', rows:30, cols:120,
        pid:null, running:false, exitCode:null, cursor:'0' };
      window.api = {
        driverInstanceId:'local-one',
        async subscribeEvents(handler) { window.onEvent = handler; return () => { window.unsubscribed++; }; },
        async invoke(operation, args) {
          calls.push({operation,args}); let output;
          if (operation === 'local.status') output = {...window.localStatus};
          else if (operation === 'local.restart') {
            window.localStatus = {...window.localStatus,shellId:'shell-one',pid:123,running:true};
            output = {status:{...window.localStatus}};
          } else if (operation === 'local.read') {
            if (!window.localStatus.shellId) throw new Error('read cannot start a shell');
            output = {shellId:window.localStatus.shellId,data:btoa('local selection text\\r\\n'),cursor:'0',gap:false,more:false,closed:!window.localStatus.running};
          } else if (operation === 'local.resize') {
            if (!window.localStatus.running) throw new Error('resize before explicit startup');
            output = {ok:true};
          } else output = {ok:true};
          return {operationId:'local-op',status:'completed',output,error:null,proposalId:null,waitExpired:false};
        },
      };
      window.view = frontend.mount(container, api);
    })()`);
    await waitForValue(session.client, "calls.some(call=>call.operation==='local.resize')");
    const first = await run("calls.map(call=>call.operation)");
    assert.equal(first.filter(name => name === "local.restart").length, 1);
    assert.ok(first.indexOf("local.restart") < first.indexOf("local.read"));
    assert.ok(first.indexOf("local.restart") < first.indexOf("local.resize"));
    await run("window.root=container.firstElementChild.shadowRoot; root.querySelector('.xterm-helper-textarea').focus()");
    for (const type of ["keyDown", "keyUp"]) await session.client.send("Input.dispatchKeyEvent", {type,key:"A",code:"KeyA",windowsVirtualKeyCode:65,modifiers:10});
    await waitForValue(session.client, "calls.some(call=>call.operation==='local.select_lines'&&call.args.text.includes('local selection text'))");
    assert.equal(await run("calls.filter(call=>call.operation==='local.select_lines').at(-1).args.shellId"), "shell-one");
    for (const type of ["keyDown", "keyUp"]) await session.client.send("Input.dispatchKeyEvent", {type,key:"Escape",code:"Escape",windowsVirtualKeyCode:27});
    await waitForValue(session.client, "calls.filter(call=>call.operation==='local.select_lines').at(-1)?.args.text===''");
    for (const running of [true, false]) {
      await run(`view.unmount(); window.localStatus.running=${running}; calls.length=0; view=frontend.mount(container,api);`);
      await waitForValue(session.client, "calls.some(call=>call.operation==='local.read')");
      assert.equal(await run("calls.some(call=>call.operation==='local.restart')"), false);
    }
    await run("view.unmount()");
    assert.equal(await run("window.unsubscribed"), 3);
  } finally {
    await session.close();
  }
});
