import { assertFreshDriverBuild } from "./support/browser-harness.mjs";
/** Real Serial bundle: one console mode controls presentation and backend capabilities. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { evaluate, findBrowser, openSession, waitForValue } from "./support/browser-harness.mjs";

test("Serial view switches synchronize backend mode without reconnecting or losing output", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshDriverBuild("serial");
  const artifact = new URL("../crates/coterm-driver/dist/serial/frontend/index.js", import.meta.url);
  for (const name of ["index.ts", "console-settings.ts"]) {
    assert.ok(fs.statSync(artifact).mtimeMs >= fs.statSync(new URL(`../crates/coterm-driver/frontend/serial/src/${name}`, import.meta.url)).mtimeMs);
  }
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({browserPath}); const {client} = session;
  const run = source => evaluate(client, source);
  const wait = source => waitForValue(client, source);
  const switchTo = async index => {
    await run(`root.querySelectorAll('.modes button')[${index}].click()`);
    await wait(`root.querySelectorAll('.modes button')[${index}].getAttribute('aria-pressed')==='true'&&!root.querySelector('.connect-button').disabled`);
  };
  try {
    await client.send("Page.enable"); await client.send("Runtime.enable");
    await client.send("Emulation.setFocusEmulationEnabled", {enabled:true});
    await run(`(async()=>{
      document.body.style.cssText='margin:0;width:1000px;height:760px';window.calls=[];window.submitted=[];window.saved=[];
      window.module=await import(${JSON.stringify(moduleURL)});
      window.receipt=output=>({status:'completed',output,error:null});
      window.i18n={locale:'zh-CN',subscribe(){return()=>{}}};
      window.frontend=module.configure(document.body,{i18n,invoke:async()=>receipt({ports:[{portName:'COM6'}]}),submit:async config=>{submitted.push(config)}});
      window.root=document.body.firstElementChild.shadowRoot;
    })()`);
    await wait(`root.querySelector('button[type=submit]').disabled===false`);
    assert.equal(await run(`root.querySelector('[name=mode]')===null`), true);
    assert.equal(await run(`root.querySelector('[name=prompt]')===null`), true);
    await run(`root.querySelector('form').dispatchEvent(new Event('submit',{cancelable:true}))`);
    await wait(`submitted.length===1`);
    assert.equal(await run(`submitted[0].mode`), "shell");
    assert.equal(await run(`submitted[0].prompt`), "# ");
    await run(`frontend.unmount();window.config={...submitted[0],prompt:'root# '};window.commandQueue=0;
      window.serialSnapshot=()=>({config:{...config},connected:true,connectionId:'first',signals:{dtr:false,rts:false},rxBytes:30,txBytes:0,commandQueue});
      window.api={driverInstanceId:'shell',i18n,
        async saveConfiguration(value){calls.push({operation:'save',args:{...value}});if(window.failSave)throw new Error('save denied');saved.push({...value})},
        async invoke(operation,args){calls.push({operation,args});
          if(operation==='serial.status')return receipt(serialSnapshot());
          if(operation==='serial.read')return receipt({text:args.cursor?'':'reply one\\r\\nreply two\\r\\nroot# ',bytes:args.cursor?0:30,cursor:1,gap:false,closed:false,more:false,connectionId:'first'});
          if(operation==='serial.read_hex')return receipt({hex:'',bytes:0,cursor:1,gap:false,closed:false,more:false,connectionId:'first'});
          if(operation==='serial.set_selection')return receipt({ok:true});
          if(operation==='serial.configure'){if(window.rejectMode)throw new Error('mode denied');Object.assign(config,args);return receipt(serialSnapshot())}
          throw new Error('Unexpected operation '+operation);
        },async subscribeEvents(fn){window.event=fn;return()=>{}}};
      window.mount=()=>{window.frontend=module.mount(document.body,api);window.root=document.body.firstElementChild.shadowRoot};mount();`);
    await wait(`root.querySelector('.connect-button').dataset.connected==='true'`);
    await wait(`root.querySelector('.xterm-rows').textContent.includes('reply one')`);
    const originalOutput = await run(`root.querySelector('.xterm-rows').textContent`);
    await run(`root.querySelector('.settings-toggle').click()`);
    assert.equal(await run(`root.querySelector('[name=mode]')===null`), true);
    assert.equal(await run(`root.querySelector('[name=prompt]').closest('label').hidden`), false);
    assert.equal(await run(`root.querySelector('[name=prompt]').value`), "root# ");
    await run(`root.querySelector('.settings-toggle').click();root.querySelector('.xterm-helper-textarea').focus()`);
    for (const type of ["keyDown", "keyUp"]) await client.send("Input.dispatchKeyEvent", {type,key:"A",code:"KeyA",windowsVirtualKeyCode:65,modifiers:10});
    await wait(`calls.some(call=>call.operation==='serial.set_selection'&&call.args.text.includes('reply one'))`);
    await switchTo(1);
    assert.equal(await run(`config.mode`), "stream");
    assert.equal(await run(`saved.at(-1).mode`), "stream");
    assert.equal(await run(`config.prompt`), "root# ");
    assert.equal(await run(`root.querySelector('[name=prompt]').closest('label').hidden`), true);
    assert.equal(await run(`root.querySelector('.uart-output').textContent.includes('reply one')`), true);

    await run(`window.rejectMode=true;root.querySelectorAll('.modes button')[0].click()`);
    await wait(`root.querySelector('.coterm-toast')?.textContent.includes('mode denied')&&!root.querySelector('.connect-button').disabled`);
    assert.equal(await run(`root.querySelector('.uart').hidden`), false, "backend rejection retains the current view");
    await run(`window.rejectMode=false;window.commandQueue=1;window.event({name:'serial.status',payload:{}})`);
    const beforeBusy = await run(`calls.filter(call=>call.operation==='serial.configure').length`);
    await run(`root.querySelectorAll('.modes button')[0].click()`);
    await wait(`Array.from(root.querySelectorAll('.coterm-toast')).some(node=>node.textContent.includes('命令队列'))`);
    assert.equal(await run(`calls.filter(call=>call.operation==='serial.configure').length`), beforeBusy);
    await run(`window.commandQueue=0;window.event({name:'serial.status',payload:{}})`);
    await switchTo(0);
    assert.equal(await run(`config.mode`), "shell");
    assert.equal(await run(`root.querySelector('.xterm-rows').textContent`), originalOutput);
    assert.equal(await run(`root.querySelector('.xterm-selection').children.length`), 0);

    await run(`window.failSave=true`);
    await switchTo(1);
    await wait(`Array.from(root.querySelectorAll('.coterm-toast')).some(node=>node.textContent.includes('配置保存失败'))`);
    assert.equal(await run(`config.mode`), "stream");
    assert.equal(await run(`saved.at(-1).mode`), "shell", "failed persistence keeps the prior saved configuration");
    const beforeRetry = await run(`calls.filter(call=>call.operation==='serial.configure').length`);
    await run(`window.failSave=false`);
    await switchTo(1);
    await wait(`saved.at(-1).mode==='stream'`);
    assert.equal(await run(`calls.filter(call=>call.operation==='serial.configure').length`), beforeRetry);
    await run(`frontend.unmount();mount()`);
    await wait(`!root.querySelector('.uart').hidden&&root.querySelector('.connect-button').dataset.connected==='true'`);
    assert.equal(await run(`root.querySelectorAll('.modes button')[1].getAttribute('aria-pressed')`), "true");
    assert.equal(await run(`calls.some(call=>['serial.connect','serial.disconnect','serial.command','serial.write','serial.write_hex'].includes(call.operation))`), false);
    await run(`frontend.unmount()`);
  } finally { await session.close(); }
});
