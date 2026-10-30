/** Real desktop UI + Tauri contract mock: workspace groups, drag/drop, cards and recovery. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { assertFreshBuild, findBrowser, openSession, evaluate, waitForValue, startStaticServer } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("workspace groups manage and move live/saved instances without reconnecting", async t => {
  const config = JSON.parse(fs.readFileSync(new URL("../apps/desktop/src-tauri/tauri.conf.json", import.meta.url)));
  assert.equal(config.app.windows[0].dragDropEnabled, false, "WebView2 must allow HTML drag/drop");
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; desktop workspace was not browser-verified"); return; }
  assertFreshBuild();
  const scenario = structuredClone(defaultScenario);
  scenario.catalog.issues = [];
  scenario.workspace = {
    groups:[{id:"default",name:"Default workspace"}],
    entries:[
      {configRef:"ref-live",pluginId:"ssh",instanceId:"inst-1",label:"Prod-Server",workspaceId:"default"},
      {configRef:"ref-saved",pluginId:"ssh",instanceId:null,label:"Standby",workspaceId:"default"},
    ], layout:{activeConfigRef:"ref-live",instancesCollapsed:false,activeWorkspaceId:"default"},
    allowedOperations:[],historyPolicy:"session_only",persistence:"windows_dpapi",
  };
  const server = await startStaticServer();
  const session = await openSession({ browserPath, totalTimeoutMs:60_000 });
  const { client } = session;
  const run = source => evaluate(client, source);
  const wait = (expression,label) => waitForValue(client,expression,{label});
  const errors = [];
  client.on("Runtime.exceptionThrown", params => errors.push(params.exceptionDetails.exception?.description ?? params.exceptionDetails.text));
  const confirm = async () => {
    await wait(`document.querySelector('dialog.host-secret-dialog[open]')`, "confirmation dialog");
    await run(`document.querySelector('dialog.host-secret-dialog button[type=submit]').click()`);
  };
  const move = async (selector, workspaceId) => {
    const data = await run(`(() => {
      const row=document.querySelector(${JSON.stringify(selector)}), transfer=new DataTransfer();
      row.dispatchEvent(new DragEvent('dragstart',{dataTransfer:transfer,bubbles:true}));
      return {items:[...transfer.types].map(mimeType=>({mimeType,data:transfer.getData(mimeType)})),dragOperationsMask:16};
    })()`);
    assert.equal(data.items[0]?.mimeType, "application/x-coterm-workspace-entry");
    const point = await run(`(() => {const r=document.querySelector('[data-workspace-id="'+${JSON.stringify(workspaceId)}+'"] > div').getBoundingClientRect();return {x:r.left+30,y:r.top+15};})()`);
    for (const type of ["dragEnter","dragOver","drop"]) await client.send("Input.dispatchDragEvent", {type,...point,data});
    await run(`document.querySelector(${JSON.stringify(selector)})?.dispatchEvent(new DragEvent('dragend',{bubbles:true}))`);
  };
  try {
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Emulation.setDeviceMetricsOverride", {width:1280,height:800,deviceScaleFactor:1,mobile:false});
    await client.send("Page.addScriptToEvaluateOnNewDocument", {source:makeHostMockSource(scenario)});
    await client.send("Page.navigate", {url:server.url});
    await wait(`!!document.querySelector('[data-instance-id="inst-1"]') && [...document.querySelectorAll('main div')].some(el=>el.shadowRoot?.querySelector('.xterm'))`, "real SSH workspace mounted");
    await run(`window.sshHost=[...document.querySelectorAll('main div')].find(el=>el.shadowRoot?.querySelector('.xterm'));
      window.initialSubscriptions=__cotermTest.countCalls('subscribe_plugin_events');`);
    assert.equal(await run(`document.querySelector('[role=toolbar]').textContent.includes('Prod-Server')`), false);
    assert.equal(await run(`document.querySelector('[role=toolbar]').textContent.includes('MCP')`), false);
    assert.equal(await run(`document.querySelector('[role=toolbar] #toggle-instances-button')`), null);
    assert.equal(await run(`document.querySelector('aside[aria-label="Installed plugins"] h1')`), null);
    assert.equal(await run(`document.querySelectorAll('#toggle-instances-button').length`), 1);
    assert.equal(await run(`document.querySelector('[data-instance-id] [data-plugin-icon]').innerHTML === document.querySelector('nav[aria-label="Plugin list"] [data-plugin-icon]').innerHTML`), true);
    assert.ok(await run(`parseFloat(getComputedStyle(document.querySelector('[data-saved-ref]')).borderRadius) > 0`), "saved cards use the same rounded layout");
    const actions = `[data-instance-id="inst-1"] > div`;
    await client.send("Input.dispatchMouseEvent", {type:"mouseMoved",x:900,y:100});
    assert.equal(await run(`getComputedStyle(document.querySelector('${actions}')).opacity`), "0");
    const hover = await run(`(() => {const r=document.querySelector('[data-instance-id]').getBoundingClientRect();return {x:r.left+15,y:r.top+15};})()`);
    await client.send("Input.dispatchMouseEvent", {type:"mouseMoved",...hover});
    await wait(`getComputedStyle(document.querySelector('${actions}')).opacity === '1'`, "hover reveals horizontal actions");
    assert.ok(await run(`(() => {const [a,b]=document.querySelectorAll('${actions} button');const x=a.getBoundingClientRect(),y=b.getBoundingClientRect();return Math.abs(x.top-y.top)<1 && y.left>=x.right;})()`));
    await client.send("Input.dispatchMouseEvent", {type:"mouseMoved",x:900,y:100});
    await run(`document.querySelector('${actions} button').focus()`);
    assert.equal(await run(`getComputedStyle(document.querySelector('${actions}')).opacity`), "1", "keyboard focus reveals actions too");

    await run(`document.querySelector('button[aria-label="New workspace"]').click()`);
    await wait(`!!document.querySelector('dialog.host-secret-dialog input')`, "workspace name prompt");
    await run(`__cotermTest.submitSecret('Lab')`);
    await wait(`document.querySelectorAll('[data-workspace-id]').length === 2`, "workspace created");
    const lab = await run(`__cotermTest.workspace().groups.find(group=>group.name==='Lab').id`);
    await move('[data-instance-id="inst-1"]', lab);
    await wait(`document.querySelector('[data-instance-id="inst-1"]').closest('[data-workspace-id]').dataset.workspaceId === ${JSON.stringify(lab)}`, "live instance moved");
    await move('[data-saved-ref="ref-saved"]', lab);
    await wait(`document.querySelector('[data-saved-ref="ref-saved"]').closest('[data-workspace-id]').dataset.workspaceId === ${JSON.stringify(lab)}`, "saved instance moved");
    assert.equal(await run(`__cotermTest.workspace().entries.length`), 2);
    assert.equal(await run(`sshHost.isConnected && __cotermTest.countCalls('subscribe_plugin_events') === initialSubscriptions`), true,
      "dragging does not remount or resubscribe the SSH frontend");
    assert.equal(await run(`__cotermTest.invocations.some(item=>['create_instance','destroy_instance'].includes(item.command) || item.args.operation==='ssh.connect')`), false);

    // Moving through the instance menu provides a keyboard-accessible alternative.
    await run(`document.querySelector('[data-instance-id="inst-1"] button[aria-haspopup=dialog]').click()`);
    await wait(`!!document.querySelector('select[aria-label="Move to workspace"]')`, "instance tools");
    await run(`(() => {const select=document.querySelector('select[aria-label="Move to workspace"]');select.value='default';select.dispatchEvent(new Event('change',{bubbles:true}));})()`);
    await wait(`document.querySelector('[data-instance-id="inst-1"]').closest('[data-workspace-id]').dataset.workspaceId === 'default'`, "keyboard alternative moved instance");
    await run(`document.querySelector('dialog button[aria-label="Close dialog"]').click()`);
    assert.equal(await run(`sshHost.isConnected && __cotermTest.countCalls('subscribe_plugin_events') === initialSubscriptions`), true);

    await run(`__cotermTest.setFailure('move_workspace_entry',true,[1])`);
    await move('[data-instance-id="inst-1"]', lab);
    await wait(`document.querySelector('main [role=alert]')?.textContent.includes('move_workspace_entry')`, "failed move displayed");
    assert.equal(await run(`document.querySelector('[data-instance-id="inst-1"]').closest('[data-workspace-id]').dataset.workspaceId`), "default", "failed move leaves membership intact");
    assert.equal(await run(`document.querySelector('button[aria-label="New workspace"]').disabled`), false);
    await run(`document.querySelector('button[aria-label="Delete workspace Lab"]').click()`);
    await wait(`!!document.querySelector('dialog.host-secret-dialog[open]')`, "delete workspace confirmation");
    await run(`__cotermTest.clickCancel()`);
    assert.equal(await run(`document.querySelectorAll('[data-workspace-id]').length`), 2);
    await run(`document.querySelector('button[aria-label="Delete workspace Lab"]').click()`);
    await confirm();
    await wait(`document.querySelectorAll('[data-workspace-id]').length === 1`, "workspace deleted");
    assert.equal(await run(`document.querySelector('[data-saved-ref="ref-saved"]').closest('[data-workspace-id]').dataset.workspaceId`), "default");
    assert.equal(await run(`sshHost.isConnected`), true, "deleting a group keeps the running connection");

    await run(`document.querySelector('[data-saved-ref="ref-saved"] button').click()`);
    await wait(`!document.querySelector('[data-saved-ref="ref-saved"]') && document.querySelectorAll('[data-instance-id]').length === 2`, "saved card becomes one live instance");
    assert.equal(await run(`__cotermTest.workspace().entries.filter(entry=>entry.configRef==='ref-saved').length`), 1, "restore reuses its saved reference");
    assert.equal(await run(`__cotermTest.countCalls('restore_saved_instance')`), 1);
    assert.equal(await run(`__cotermTest.invocations.some(item=>item.args.operation==='ssh.connect')`), false, "restoration never connects automatically");
    await run(`document.querySelector('button[aria-label="Delete instance Standby"]').click()`);
    await confirm();
    await wait(`document.querySelectorAll('[data-instance-id]').length === 1`, "instance deleted directly from its card");
    assert.equal(await run(`__cotermTest.countCalls('destroy_instance')`), 1);
    assert.equal(await run(`__cotermTest.workspace().entries.some(entry=>entry.configRef==='ref-saved')`), false);
    await run(`(() => {const select=document.getElementById('locale-select');select.value='zh-CN';select.dispatchEvent(new Event('change',{bubbles:true}));})()`);
    await wait(`!!document.querySelector('button[aria-label="新建工作区"]')`, "workspace controls translate");
    assert.ok(await run(`document.querySelector('[data-workspace-id="default"]').textContent.includes('默认工作区')`));
    assert.equal(await run(`document.documentElement.scrollWidth <= innerWidth`), true);
    assert.deepEqual(errors, []);
  } catch (error) {
    t.diagnostic(JSON.stringify(await run(`({groups:__cotermTest?.workspace(),last:__cotermTest?.invocations.slice(-5),body:document.body.innerText.slice(-1500)})`).catch(()=>null)));
    throw error;
  } finally { await session.close(); await new Promise(resolve => server.server.close(resolve)); }
});
