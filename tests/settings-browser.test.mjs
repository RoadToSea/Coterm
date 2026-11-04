/** Public Settings UI + actual host/SSH/serial bundles; only backend operations are mocked. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { assertFreshBuild, evaluate, findBrowser, openSession, rootDir, startStaticServer, waitForValue } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("categorized settings search, apply live fonts, persist and preserve mounted connections", async t => {
  const browserPath=findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge; settings UI was not verified"); return; }
  assertFreshBuild();
  const server=await startStaticServer();
  const session=await openSession({browserPath,totalTimeoutMs:90000});
  const {client}=session;
  const run=source=>evaluate(client,source);
  const wait=source=>waitForValue(client,source);
  const errors=[];
  const select=async(id,value)=>{await run(`(()=>{const input=document.getElementById(${JSON.stringify(id)});input.value=${JSON.stringify(String(value))};input.dispatchEvent(new Event('change',{bubbles:true}));})()`);};
  const search=async value=>{await run(`(()=>{const input=document.getElementById('settings-search');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,${JSON.stringify(value)});input.dispatchEvent(new Event('input',{bubbles:true}));})()`);};
  const category=async id=>{await run(`document.querySelector('[data-category=${id}]').click()`);};
  const shot=async name=>{
    const result=await client.send("Page.captureScreenshot",{format:"png"});
    fs.mkdirSync(path.join(rootDir,"target"),{recursive:true});
    fs.writeFileSync(path.join(rootDir,"target",name),Buffer.from(result.data,"base64"));
  };
  try {
    await client.send("Page.enable"); await client.send("Runtime.enable");
    client.on("Runtime.exceptionThrown", event=>errors.push(event.exceptionDetails.exception?.description??event.exceptionDetails.text));
    await client.send("Page.addScriptToEvaluateOnNewDocument",{source:makeHostMockSource(defaultScenario)});
    await client.send("Emulation.setDeviceMetricsOverride",{width:1280,height:800,deviceScaleFactor:1,mobile:false});
    await client.send("Page.navigate",{url:server.url});
    await wait(`(()=>{const host=[...document.querySelectorAll('main div')].find(node=>node.shadowRoot?.querySelector('.xterm'));if(!host)return false;window.settingsSsh=host;window.settingsTerminal=host.shadowRoot.querySelector('.xterm');return !!host.shadowRoot.querySelector('.bar span')?.textContent.includes('Connected');})()`);
    assert.equal(await run(`!!document.getElementById('locale-select')`),false,'language moves into settings');
    await run(`window.beforeSubscriptions=__cotermTest.calls.filter(c=>c==='subscribe_plugin_events').length;window.beforeProtocol=__cotermTest.invocations.filter(c=>['ssh.connect','ssh.disconnect','ssh.write'].includes(c.args.operation)).length;document.getElementById('open-settings').focus();document.getElementById('open-settings').click()`);
    await wait(`document.getElementById('app-settings-dialog')?.open`);
    await wait(`document.activeElement.id==='settings-search'`);
    assert.equal(await run(`document.querySelectorAll('[data-setting]').length`),7);
    await shot('settings-light.png');
    await category('general');
    assert.equal(await run(`document.querySelectorAll('[data-setting]').length`),1);
    await search('font');
    await wait(`document.querySelectorAll('[data-setting]').length===2`);
    assert.equal(await run(`!!document.getElementById('terminal-font-size')`),true,'search spans all categories');
    await search('字体');
    await wait(`document.querySelectorAll('[data-setting]').length===2`);
    await search('nothing-matches-this');
    await wait(`document.querySelector('[data-slot=empty-title]')?.textContent==='No settings found'`);
    await category('appearance');
    assert.equal(await run(`document.getElementById('settings-search').value`),'');
    await select('interface-font-size',18);
    await wait(`parseFloat(getComputedStyle(document.documentElement).fontSize)>20`);
    await run(`document.querySelector('#settings-theme-switcher button[aria-label="Dark mode"]').click()`);
    await wait(`document.documentElement.dataset.theme==='dark'`);
    assert.equal(await run(`document.querySelector('#theme-switcher button[aria-label="Dark mode"]').dataset.state`),'on');
    await category('terminal');
    await select('terminal-font-size',20);
    await select('terminal-line-height',1.4);
    await run(`document.getElementById('cursor-blink').click()`);
    await wait(`getComputedStyle(settingsSsh.shadowRoot.querySelector('.xterm-char-measure-element')).fontSize==='20px'`);
    assert.equal(await run(`settingsTerminal===settingsSsh.shadowRoot.querySelector('.xterm')`),true);
    assert.equal(await run(`__cotermTest.calls.filter(c=>c==='subscribe_plugin_events').length===beforeSubscriptions`),true);
    assert.equal(await run(`__cotermTest.invocations.filter(c=>['ssh.connect','ssh.disconnect','ssh.write'].includes(c.args.operation)).length===beforeProtocol`),true);
    assert.deepEqual(await run(`JSON.parse(localStorage.getItem('coterm.preferences.v1'))`),{interfaceFontSize:18,terminalFontSize:20,terminalLineHeight:1.4,cursorBlink:false});
    await shot('settings-dark-terminal.png');
    await category('general'); await select('locale-select','zh-CN');
    await wait(`document.getElementById('app-settings-dialog').getAttribute('aria-label')==='设置'`);
    await shot('settings-zh.png');
    await category('terminal');
    for (const width of [768,360]) {
      await client.send("Emulation.setDeviceMetricsOverride",{width,height:700,deviceScaleFactor:1,mobile:false});
      await wait(`document.documentElement.scrollWidth<=innerWidth`);
      assert.equal(await run(`(()=>{const d=document.getElementById('app-settings-dialog');return d.scrollWidth<=d.clientWidth&&d.getBoundingClientRect().right<=innerWidth;})()`),true);
      assert.equal(await run(`document.getElementById('open-settings').getBoundingClientRect().right<=innerWidth`),true,'settings entry remains reachable with a larger font');
      await shot(`settings-narrow-${width}.png`);
    }
    await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await wait(`!document.getElementById('app-settings-dialog')`);
    assert.equal(await run(`document.activeElement.id`),'open-settings');

    // Saved preferences apply before new plugin views mount, including the actual serial frontend.
    await client.send('Page.reload');
    await wait(`document.getElementById('open-settings')&&document.documentElement.lang==='zh-CN'`);
    assert.equal(await run(`getComputedStyle(document.documentElement).getPropertyValue('--coterm-terminal-font-size').trim()`),'20px');
    const artifact=fs.readFileSync(path.join(rootDir,'plugins/plugin-serial/dist/frontend/index.js'));
    await run(`(async()=>{
      const container=document.createElement('div');container.style.cssText='position:fixed;inset:80px 20px 40px 80px';document.body.append(container);
      window.settingsSerialCalls=[];
      const receipt=output=>({status:'completed',output,error:null});
      const api={invoke:async(operation,args)=>{settingsSerialCalls.push(operation);
        if(operation==='serial.status')return receipt({config:{port:'COM1',baudRate:115200,dataBits:8,stopBits:1,parity:'none',dtr:false,rts:false},connected:true,connectionId:'settings',signals:{dtr:false,rts:false},rxBytes:0,txBytes:0,commandQueue:0});
        if(operation==='serial.read')return receipt({text:args.cursor?'':'Saved fonts\\r\\n',bytes:13,cursor:1,gap:false,closed:false,more:false,connectionId:'settings'});
        return receipt({});
      },subscribeEvents:async()=>()=>{}};
      window.settingsSerialView=(await import('data:text/javascript;base64,${artifact.toString('base64')}')).mount(container,api);
      window.settingsSerial=container.firstElementChild;
      window.settingsSerialTerminal=settingsSerial.shadowRoot.querySelector('.xterm');
    })()`);
    await wait(`getComputedStyle(settingsSerial.shadowRoot.querySelector('.xterm-char-measure-element')).fontSize==='20px'`);
    await run(`document.getElementById('open-settings').click()`);
    await wait(`document.getElementById('app-settings-dialog')?.open`);
    await run(`document.querySelector('.settings-footer button').click()`);
    await wait(`getComputedStyle(settingsSerial.shadowRoot.querySelector('.xterm-char-measure-element')).fontSize==='13px'`);
    assert.equal(await run(`settingsSerialTerminal===settingsSerial.shadowRoot.querySelector('.xterm')`),true);
    assert.equal(await run(`settingsSerialCalls.some(op=>['serial.connect','serial.disconnect','serial.write','serial.write_hex'].includes(op))`),false);
    assert.equal(await run(`document.documentElement.lang`),'zh-CN','reset affects font/cursor preferences only');
    assert.equal(await run(`document.documentElement.dataset.theme`),'dark');
    await category('terminal');
    await select('terminal-font-size',17);
    await wait(`getComputedStyle(settingsSerial.shadowRoot.querySelector('.uart-output')).fontSize==='17px'`);
    await run(`(()=>{const container=settingsSerial.parentElement;settingsSerialView.unmount();container.remove();})()`);
    // Storage messages synchronize display preferences from another window; invalid data is bounded.
    await run(`localStorage.setItem('coterm.preferences.v1',JSON.stringify({interfaceFontSize:16,terminalFontSize:18,terminalLineHeight:1.2,cursorBlink:true}));window.dispatchEvent(new StorageEvent('storage',{key:'coterm.preferences.v1'}))`);
    await wait(`document.getElementById('terminal-font-size').value==='18'`);
    await run(`localStorage.setItem('coterm.preferences.v1','invalid');window.dispatchEvent(new StorageEvent('storage',{key:'coterm.preferences.v1'}))`);
    await wait(`document.getElementById('terminal-font-size').value==='13'`);
    assert.deepEqual(errors,[]);
  } catch(error) {
    t.diagnostic(JSON.stringify(await run(`({body:document.body.innerText.slice(-1600),storage:localStorage.getItem('coterm.preferences.v1')})`).catch(()=>null)));
    throw error;
  } finally { await session.close(); await new Promise(resolve=>server.server.close(resolve)); }
});
