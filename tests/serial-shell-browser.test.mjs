/** Real Serial bundle: explicit data mode, prompt draft and human xterm selection. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { evaluate, findBrowser, openSession, waitForValue } from "./support/browser-harness.mjs";

test("Serial data mode is explicit and only Shell publishes human selection", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  const artifact = new URL("../plugins/plugin-serial/dist/frontend/index.js", import.meta.url);
  for (const name of ["index.ts", "i18n.ts", "console-settings.ts"]) {
    assert.ok(fs.statSync(artifact).mtimeMs >= fs.statSync(new URL(`../plugins/plugin-serial/frontend/src/${name}`, import.meta.url)).mtimeMs);
  }
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({browserPath}); const {client} = session;
  const run = source => evaluate(client, source);
  const wait = source => waitForValue(client, source);
  const key = async (key, code, windowsVirtualKeyCode, modifiers = 0) => {
    for (const type of ["keyDown", "keyUp"]) await client.send("Input.dispatchKeyEvent", {type,key,code,windowsVirtualKeyCode,modifiers});
  };
  try {
    await client.send("Page.enable"); await client.send("Runtime.enable");
    await client.send("Emulation.setFocusEmulationEnabled", {enabled:true});
    await run(`(async()=>{
      document.body.style.cssText='margin:0;width:1000px;height:760px';window.calls=[];window.submitted=[];
      window.module=await import(${JSON.stringify(moduleURL)});
      window.receipt=output=>({status:'completed',output,error:null});
      window.locale='zh-CN';window.i18n={get locale(){return locale},subscribe(fn){window.translate=fn;return()=>{}}};
      window.frontend=module.configure(document.body,{i18n,invoke:async()=>receipt({ports:[{portName:'COM6'}]}),submit:async config=>{submitted.push(config)}});
      window.root=document.body.firstElementChild.shadowRoot;
    })()`);
    await wait(`root.querySelector('button[type=submit]').disabled===false`);
    assert.equal(await run(`root.querySelector('[name=mode]').value`), "stream");
    assert.equal(await run(`root.querySelector('[name=prompt]').disabled`), true);
    await run(`root.querySelector('[name=mode]').value='shell';root.querySelector('[name=mode]').dispatchEvent(new Event('change'));root.querySelector('[name=prompt]').value='';root.querySelector('form').dispatchEvent(new Event('submit',{cancelable:true}))`);
    await wait(`root.querySelector('.error').textContent.includes('提示符')`);
    assert.equal(await run(`submitted.length`), 0);
    await run(`root.querySelector('[name=prompt]').value='root# ';root.querySelector('form').dispatchEvent(new Event('submit',{cancelable:true}))`);
    await wait(`submitted.length===1`);
    assert.equal(await run(`submitted[0].mode`), "shell"); assert.equal(await run(`submitted[0].prompt`), "root# ");
    await run(`frontend.unmount();window.connected=true;window.config={...submitted[0]};window.connection='first';
      window.api={pluginInstanceId:'shell',i18n,async invoke(operation,args){calls.push({operation,args});
        if(operation==='serial.status')return receipt({config,connected,connectionId:connection,signals:{dtr:false,rts:false},rxBytes:0,txBytes:0,commandQueue:0});
        if(operation==='serial.read')return receipt({text:'reply one\\r\\nreply two\\r\\nroot# ',bytes:30,cursor:1,gap:false,closed:false,more:false,connectionId:connection});
        if(operation==='serial.set_selection')return receipt({ok:true});
        if(operation==='serial.disconnect'){connected=false;return receipt({})}
        if(operation==='serial.configure'){Object.assign(config,args);return receipt({})}
        if(operation==='serial.connect'){connected=true;connection='second';return receipt({})}
        throw new Error('Unexpected operation '+operation);
      },async subscribeEvents(fn){window.event=fn;return()=>{}}};
      window.frontend=module.mount(document.body,api);window.root=document.body.firstElementChild.shadowRoot;`);
    await wait(`root.querySelector('.connect-button').dataset.connected==='true'`);
    await run(`root.querySelector('.xterm-helper-textarea').focus()`); await key("A","KeyA",65,10);
    await wait(`calls.some(call=>call.operation==='serial.set_selection'&&call.args.text.includes('reply one'))`);
    assert.equal(await run(`calls.filter(call=>call.operation==='serial.set_selection').at(-1).args.connectionId`), "first");
    await key("Escape","Escape",27);
    await wait(`calls.filter(call=>call.operation==='serial.set_selection').at(-1).args.text===''`);
    await run(`root.querySelector('.connect-button').click()`);
    await wait(`root.querySelector('.connect-button').dataset.connected==='false'`);
    await run(`root.querySelector('.settings-toggle').click();root.querySelector('[name=mode]').value='stream';root.querySelector('[name=mode]').dispatchEvent(new Event('change'))`);
    assert.equal(await run(`calls.filter(call=>call.operation==='serial.configure').length`), 0, "editing does not apply configuration");
    if (process.env.COTERM_SERIAL_SHELL_SCREENSHOT_DIR) {
      await client.send("Emulation.setDeviceMetricsOverride", {width:360,height:760,deviceScaleFactor:1,mobile:false});
      await run(`document.body.style.width='360px'`);
      await wait(`getComputedStyle(root.querySelector('.settings-panel')).transform==='matrix(1, 0, 0, 1, 0, 0)'`);
      await run(`root.querySelector('.settings-panel').scrollTop=0;new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
      const shot = await client.send("Page.captureScreenshot", {format:"png"});
      fs.mkdirSync(process.env.COTERM_SERIAL_SHELL_SCREENSHOT_DIR, {recursive:true});
      fs.writeFileSync(path.join(process.env.COTERM_SERIAL_SHELL_SCREENSHOT_DIR,"serial-shell-settings-360.png"),Buffer.from(shot.data,"base64"));
    }
    await run(`root.querySelector('.settings-toggle').click();root.querySelector('.connect-button').click()`);
    await wait(`root.querySelector('.connect-button').dataset.connected==='true'`);
    assert.equal(await run(`calls.find(call=>call.operation==='serial.configure').args.mode`), "stream");
    assert.equal(await run(`calls.find(call=>call.operation==='serial.configure').args.prompt`), null);
    const before = await run(`calls.filter(call=>call.operation==='serial.set_selection').length`);
    await run(`root.querySelector('.xterm-helper-textarea').focus()`); await key("A","KeyA",65,10);
    await run(`new Promise(resolve=>setTimeout(resolve,200))`);
    assert.equal(await run(`calls.filter(call=>call.operation==='serial.set_selection').length`), before);
    assert.equal(await run(`calls.filter(call=>call.operation==='serial.command'||call.operation==='serial.write').length`), 0);
    await run(`frontend.unmount()`);
  } finally { await session.close(); }
});
