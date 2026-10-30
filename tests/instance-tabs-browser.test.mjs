/** Real host + real plugin frontends; only the Tauri backend contract is mocked. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { assertFreshBuild, findBrowser, openSession, evaluate, waitForValue, startStaticServer } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("inline protocol configuration and instance tabs preserve live frontends and separate view/lifecycle actions", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; instance tabs were not browser-verified"); return; }
  assertFreshBuild();
  for (const plugin of ["ssh", "serial"]) {
    assert.ok(fs.statSync(new URL(`../plugins/plugin-${plugin}/dist/frontend/index.js`, import.meta.url)).mtimeMs >=
      fs.statSync(new URL(`../plugins/plugin-${plugin}/frontend/src/index.ts`, import.meta.url)).mtimeMs, "rebuild the actual plugin frontend");
  }
  const scenario = structuredClone(defaultScenario);
  scenario.installPreview = {token:"tab-refresh-test",id:"test.tabs",name:"Tabs test",version:"0.1.0",platform:"windows-x64"};
  scenario.catalog.issues = [];
  scenario.catalog.plugins.push(
    {id:"serial",name:"Serial",version:"0.1.0",hasFrontend:true,running:false},
    {id:"local",name:"Local",version:"0.1.0",hasFrontend:false,running:false},
  );
  const server = await startStaticServer();
  const session = await openSession({ browserPath, totalTimeoutMs:90_000 });
  const { client } = session;
  const run = source => evaluate(client, source);
  const wait = (expression, label) => waitForValue(client, expression, {label});
  const errors = [];
  client.on("Runtime.exceptionThrown", event => errors.push(event.exceptionDetails.exception?.description ?? event.exceptionDetails.text));
  const selectTab = id => run(`document.getElementById('instance-tab-'+${JSON.stringify(id)}).click()`);
  const keyboard = async (key, code, windowsVirtualKeyCode) => {
    await client.send("Input.dispatchKeyEvent", {type:"keyDown",key,code,windowsVirtualKeyCode});
    await client.send("Input.dispatchKeyEvent", {type:"keyUp",key,code,windowsVirtualKeyCode});
  };
  try {
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Emulation.setDeviceMetricsOverride", {width:1280,height:800,deviceScaleFactor:1,mobile:false});
    await client.send("Page.addScriptToEvaluateOnNewDocument", {source:makeHostMockSource(scenario)});
    await client.send("Page.navigate", {url:server.url});
    await wait(`document.querySelector('#instance-panel-inst-1 div') && [...document.querySelectorAll('#instance-panel-inst-1 div')].some(el=>el.shadowRoot?.querySelector('.bar span')?.textContent==='Connected')`, "initial SSH frontend connected");
    await run(`window.originalHost=[...document.querySelectorAll('#instance-panel-inst-1 div')].find(el=>el.shadowRoot?.querySelector('.xterm'));
      window.originalRoot=originalHost.shadowRoot; window.originalTerminal=originalRoot.querySelector('.xterm');
      window.baseSubscriptions=__cotermTest.countCalls('subscribe_plugin_events');
      window.configuration=()=>[...document.querySelectorAll('#plugin-configuration div')].find(el=>el.shadowRoot?.querySelector('form'))?.shadowRoot;`);
    assert.equal(await run(`document.querySelectorAll('[role=tab]').length`), 1);
    await run(`document.querySelector('button[aria-label="Manage SSH"]').click()`);
    await wait(`!!configuration()?.querySelector('form')`, "SSH inline connection form");
    assert.equal(await run(`document.querySelector('dialog[open]')`), null, "new instance configuration does not open a dialog");
    assert.equal(await run(`document.getElementById('instance-panel-inst-1').hidden && originalHost.isConnected`), true);
    assert.equal(await run(`/Plugin operations|Advanced JSON|Configuration \(JSON\)/.test(document.querySelector('main').textContent)`), false);
    assert.equal(await run(`__cotermTest.countCalls('list_operations')`), 0, "ordinary navigation no longer fetches operation debugger catalogs");
    assert.ok(await run(`[...configuration().querySelectorAll('input,button')].every(el=>parseFloat(getComputedStyle(el).borderRadius)>0)`));

    // Failed creation keeps the same form and values; global busy also blocks other UI submissions.
    await run(`window.formBefore=configuration().querySelector('form');
      (()=>{const fields=configuration().querySelectorAll('input');fields[0].value='lab.example';fields[2].value='tester';})();
      window.creationGate=__cotermTest.gate('create_instance');formBefore.requestSubmit();`);
    await wait(`creationGate.size()===1 && document.querySelector('#plugin-configuration fieldset').getAttribute('aria-busy')==='true'`, "creation in flight");
    assert.equal(await run(`document.querySelector('#plugin-configuration fieldset > div').inert`), true);
    await run(`formBefore.requestSubmit()`);
    assert.equal(await run(`__cotermTest.countCalls('create_instance')`), 1, "duplicate submit is suppressed");
    await run(`creationGate.fail('simulated create failure')`);
    await wait(`configuration()?.querySelector('.error').textContent.includes('simulated create failure')`, "creation failure is visible");
    assert.equal(await run(`formBefore === configuration().querySelector('form') && configuration().querySelector('input').value==='lab.example'`), true);
    await run(`formBefore.requestSubmit()`);
    await wait(`document.querySelectorAll('[role=tab]').length===2 && !document.getElementById('plugin-configuration')`, "successful creation opens a tab");
    const created = await run(`__cotermTest.invocations.filter(x=>x.command==='create_instance').at(-1).args`);
    assert.deepEqual(created, {pluginId:"ssh",config:{host:"lab.example",port:22,username:"tester"},workspaceId:"default"});
    await wait(`__cotermTest.countCalls('subscribe_plugin_events')===baseSubscriptions+1`, "new SSH subscription");
    assert.equal(await run(`originalHost.isConnected && document.getElementById('instance-panel-inst-1').hidden`), true);

    // Switching and visiting a config page preserve the real terminal DOM and subscription.
    await selectTab("inst-1");
    await wait(`!document.getElementById('instance-panel-inst-1').hidden`, "initial tab selected");
    await run(`originalRoot.querySelector('.xterm-helper-textarea').value='pending input'`);
    await selectTab("created-1");
    await selectTab("inst-1");
    assert.equal(await run(`originalHost.isConnected && originalRoot.querySelector('.xterm')===originalTerminal && originalRoot.querySelector('.xterm-helper-textarea').value==='pending input'`), true);
    assert.ok(await run(`originalRoot.querySelector('.xterm').textContent.includes('Connected to Prod-Server')`), "terminal replay output survives navigation");
    assert.equal(await run(`__cotermTest.countCalls('subscribe_plugin_events')`), 2);
    assert.equal(await run(`__cotermTest.countCalls('unsubscribe_plugin_events')`), 0);

    // Serial uses its own rounded form, starts the stopped plugin and creates the chosen config.
    await run(`document.querySelector('button[aria-label="Manage Serial"]').click()`);
    await wait(`!!configuration()?.querySelector('select')`, "serial inline protocol fields");
    assert.ok(await run(`[...configuration().querySelectorAll('input:not([type=checkbox]),select,button')].every(el=>parseFloat(getComputedStyle(el).borderRadius)>0)`));
    assert.equal(await run(`configuration().querySelectorAll('textarea').length`), 0);
    await run(`configuration().querySelector('input').value='COM2';configuration().querySelector('form').requestSubmit()`);
    await wait(`document.querySelectorAll('[role=tab]').length===3 && document.getElementById('instance-tab-created-2')?.getAttribute('aria-selected')==='true'`, "serial created and selected");
    await wait(`__cotermTest.countCalls('subscribe_plugin_events')===3`, "serial frontend mounted");
    assert.equal(await run(`__cotermTest.countCalls('start_plugin')`), 1);
    assert.deepEqual(await run(`__cotermTest.invocations.filter(x=>x.command==='create_instance').at(-1).args.config`),
      {port:"COM2",baudRate:115200,dataBits:8,stopBits:1,parity:"none",dtr:true,rts:true});

    await run(`document.querySelector('button[aria-label="Manage Local"]').click()`);
    await wait(`document.getElementById('plugin-configuration')?.textContent.includes('does not provide a connection configuration screen')`, "headless plugin explains missing config UI");
    assert.equal(await run(`document.querySelector('#plugin-configuration textarea')`), null);
    assert.equal(await run(`document.querySelector('dialog[open]')`), null);

    // Keyboard tab navigation and scroll containment in a narrow window.
    await selectTab("inst-1");
    await run(`document.getElementById('instance-tab-inst-1').focus()`);
    await keyboard("ArrowRight", "ArrowRight", 39);
    await wait(`document.getElementById('instance-tab-created-1').getAttribute('aria-selected')==='true'`, "arrow selects next tab");
    await keyboard("End", "End", 35);
    await wait(`document.getElementById('instance-tab-created-2').getAttribute('aria-selected')==='true'`, "End selects last tab");
    await keyboard("Home", "Home", 36);
    await wait(`document.activeElement.id==='instance-tab-inst-1'`, "Home restores first tab focus");
    await client.send("Emulation.setDeviceMetricsOverride", {width:520,height:650,deviceScaleFactor:1,mobile:false});
    await selectTab("created-2");
    await wait(`document.querySelector('[role=tablist]').scrollLeft>0`, "selected last tab scrolls into view");
    assert.equal(await run(`document.documentElement.scrollWidth<=innerWidth`), true);
    await wait(`(()=>{const a=document.querySelector('[role=tablist]').getBoundingClientRect(),b=document.getElementById('instance-tab-created-2').getBoundingClientRect();return b.left>=a.left && b.right<=a.right;})()`, "selected tab stays visible after responsive sidebar resize");
    await client.send("Emulation.setDeviceMetricsOverride", {width:1280,height:800,deviceScaleFactor:1,mobile:false});

    // Closing a background tab keeps the active one; closing a view does not delete/disconnect.
    await run(`document.querySelector('button[aria-label="Close view tester@lab.example"]').click()`);
    await wait(`!document.getElementById('instance-tab-created-1')`, "background tab closed");
    assert.equal(await run(`document.getElementById('instance-tab-created-2').getAttribute('aria-selected')`), "true");
    assert.equal(await run(`__cotermTest.workspace().entries.length`), 3);
    assert.equal(await run(`__cotermTest.countCalls('destroy_instance')`), 0);
    await run(`document.querySelector('[data-instance-id="created-1"] > button').click()`);
    await wait(`document.getElementById('instance-tab-created-1')?.getAttribute('aria-selected')==='true'`, "sidebar reopens closed view");
    await run(`document.getElementById('instance-tab-created-1').focus()`);
    await keyboard("Delete", "Delete", 46);
    await wait(`!document.getElementById('instance-tab-created-1') && document.activeElement.id==='instance-tab-created-2'`, "Delete closes selected view and focuses neighbor");

    // Failed process removes its frontend, exposes recovery, then removal clears its tab.
    await run(`__cotermTest.replaceInstances([
      {pluginId:'ssh',pluginInstanceId:'inst-1',sessionId:'sess-1',label:'Prod-Server',status:'ready'},
      {pluginId:'serial',pluginInstanceId:'created-2',sessionId:'session-2',label:'COM2',status:'failed'}
    ])`);
    await wait(`document.getElementById('instance-panel-created-2')?.textContent.includes('Plugin stopped unexpectedly')`, "failed instance recovery");
    assert.equal(await run(`[...document.querySelectorAll('#instance-panel-created-2 div')].some(el=>!!el.shadowRoot)`), false);
    await run(`__cotermTest.replaceInstances([{pluginId:'ssh',pluginInstanceId:'inst-1',sessionId:'sess-1',label:'Prod-Server',status:'ready'}])`);
    await wait(`!document.getElementById('instance-tab-created-2') && document.getElementById('instance-tab-inst-1').getAttribute('aria-selected')==='true'`, "runtime removal selects remaining tab");
    await run(`document.querySelector('button[aria-label="Close view Prod-Server"]').click()`);
    await wait(`document.querySelector('[role=tablist]')===null && document.querySelector('main').textContent.includes('Ready when you are')`, "last tab closed");
    await wait(`__cotermTest.countCalls('unsubscribe_plugin_events')===__cotermTest.countCalls('subscribe_plugin_events')`, "all closed views clean subscriptions");
    // A normal refresh (installing an offline package) cannot reopen explicitly closed tabs.
    await run(`document.getElementById('install-plugin-button').click()`);
    await wait(`!!document.querySelector('dialog[open]')`, "installer opened");
    await run(`[...document.querySelectorAll('dialog button')].find(button=>button.textContent==='Choose plugin package').click()`);
    await wait(`!!document.querySelector('dialog dl')`, "install preview");
    await run(`[...document.querySelectorAll('dialog button')].find(button=>button.textContent==='Install trusted package').click()`);
    await wait(`!!document.querySelector('button[aria-label="Manage Tabs test"]')`, "catalog refresh after installation");
    await run(`document.querySelector('dialog button[aria-label="Close dialog"]').click()`);
    assert.equal(await run(`document.querySelector('[role=tablist]')`), null);
    await run(`document.querySelector('[data-instance-id="inst-1"] > button').click()`);
    await wait(`document.getElementById('instance-tab-inst-1')?.getAttribute('aria-selected')==='true'`, "live instance remains available after closing last view");
    assert.equal(await run(`__cotermTest.invocations.some(x=>['ssh.connect','ssh.disconnect','serial.connect','serial.disconnect'].includes(x.args.operation))`), false);

    await run(`document.querySelector('button[aria-label="Delete instance Prod-Server"]').click()`);
    await wait(`!!document.querySelector('dialog.host-secret-dialog[open]')`, "destroy confirmation");
    await run(`document.querySelector('dialog.host-secret-dialog button[type=submit]').click()`);
    await wait(`!document.getElementById('instance-tab-inst-1')`, "destroy removes the tab");
    assert.equal(await run(`__cotermTest.countCalls('destroy_instance')`), 1);
    await run(`(()=>{const select=document.getElementById('locale-select');select.value='zh-CN';select.dispatchEvent(new Event('change',{bubbles:true}));})();
      document.querySelector('button[aria-label="管理 SSH"]').click()`);
    await wait(`configuration()?.querySelector('button').textContent==='创建 SSH 实例'`, "inline plugin form translates");
    assert.ok(await run(`document.getElementById('plugin-configuration').textContent.includes('新建实例')`));
    assert.deepEqual(errors, []);
  } catch (error) {
    t.diagnostic(JSON.stringify(await run(`({body:document.body.innerText.slice(-1800),calls:__cotermTest.invocations.slice(-8).map(x=>({command:x.command,args:x.args}))})`).catch(()=>null)));
    throw error;
  } finally { await session.close(); await new Promise(resolve => server.server.close(resolve)); }
});
