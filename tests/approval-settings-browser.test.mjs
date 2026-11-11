import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { assertFreshBuild, evaluate, findBrowser, openSession, setAppLocale, startStaticServer, waitForValue } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

async function harness(t, scenario) {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return null; }
  assertFreshBuild();
  const server = await startStaticServer();
  const session = await openSession({ browserPath });
  const { client } = session;
  const run = source => evaluate(client, source);
  const wait = expression => waitForValue(client, expression, { label: expression });
  try {
    await client.send("Page.enable"); await client.send("Runtime.enable");
    await client.send("Page.addScriptToEvaluateOnNewDocument", { source: makeHostMockSource(scenario) });
    await client.send("Page.navigate", { url: server.url });
    await wait(`!!document.getElementById('open-settings')`);
  } catch (error) {
    await session.close(); await new Promise(resolve => server.server.close(resolve)); throw error;
  }
  const input = async (id, value) => run(`(() => { const input=document.getElementById(${JSON.stringify(id)});
    Object.getOwnPropertyDescriptor(input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype,'value').set.call(input,${JSON.stringify(value)});
    input.dispatchEvent(new Event('input',{bubbles:true})); })()`);
  const click = text => run(`[...document.querySelectorAll('dialog[open] button')].find(button=>button.textContent===${JSON.stringify(text)}).click()`);
  return {client,run,wait,input,click,close:async()=>{await session.close();await new Promise(resolve=>server.server.close(resolve));}};
}

const twoInstances = () => {
  const scenario = structuredClone(defaultScenario);
  scenario.instances.push({...scenario.instances[0],driverInstanceId:"inst-2",sessionId:"sess-2",label:"Staging-Server"});
  return scenario;
};

test("independent rule groups work without instances and preserve failed saves and unsaved drafts", async t => {
  const scenario=structuredClone(defaultScenario); scenario.instances=[]; scenario.approvalPreview={ls:false};
  const session=await harness(t,scenario);if(!session)return;
  const {client,run,wait,input,click}=session;
  try {
    await run(`document.getElementById('open-settings').click()`);await click('Manage rule groups');
    await wait(`!!document.getElementById('approval-rule-groups-dialog') && !!document.querySelector('button')`);
    await wait(`[...document.querySelectorAll('dialog button')].some(button=>button.textContent==='New rule group')`);
    await click('New rule group');await input('approval-group-name','Board rules');await input('approval-pattern','^ls$');await click('Add rule');
    await run(`window.saveGate=__cotermTest.gate('save_approval_group')`);await click('Save group');await wait(`saveGate.size()===1`);
    await run(`saveGate.fail('disk unavailable')`);await wait(`document.querySelector('[role=alert]')?.textContent.includes('disk unavailable')`);
    assert.equal(await run(`document.getElementById('approval-group-name').value`),'Board rules');
    assert.equal(await run(`document.querySelector('input[aria-label="Rule 1"]').value`),'^ls$');
    assert.equal(await run(`__cotermTest.workspace().approvalRuleGroups.length`),0);
    await click('Save group');await wait(`__cotermTest.workspace().approvalRuleGroups.length===1`);
    await input('approval-preview','ls');await click('Test matching');await wait(`document.getElementById('approval-rule-groups-dialog').textContent.includes('Skips human approval')`);
    assert.deepEqual(await run(`Object.keys(__cotermTest.invocations.find(call=>call.command==='preview_approval_policy').args.policy).sort()`),['mode','patterns']);
    await click('Matching commands require approval');await click('Save group');
    await wait(`__cotermTest.workspace().approvalRuleGroups[0].mode==='approve_matches'`);
    await click('Duplicate');await click('Save group');await wait(`__cotermTest.workspace().approvalRuleGroups.length===2`);
    await input('approval-group-name','Edited copy');await click('Delete group');
    await wait(`[...document.querySelectorAll('dialog button')].some(button=>button.textContent==='Save and continue')`);
    await click('Save and continue');await wait(`__cotermTest.workspace().approvalRuleGroups.length===1`);
    await click('New rule group');await input('approval-group-name','Unsaved');
    await run(`document.querySelector('#approval-rule-groups-dialog button[aria-label="Close dialog"]').click()`);
    await wait(`[...document.querySelectorAll('dialog button')].some(button=>button.textContent==='Keep editing')`);
    await click('Keep editing');assert.equal(await run(`document.getElementById('approval-group-name').value`),'Unsaved');
    await run(`document.querySelector('[data-approval-group]').click()`);await click('Discard and continue');
    await wait(`document.getElementById('approval-group-name').value==='Board rules'`);
    await run(`document.querySelector('#approval-rule-groups-dialog button[aria-label="Close dialog"]').click()`);
    await setAppLocale(client,'zh-CN');
    await run(`document.getElementById('open-settings').click()`);await click('管理规则组');
    await wait(`document.getElementById('approval-rule-groups-dialog')?.textContent.includes('审批规则组') && !!document.getElementById('approval-group-name')`);
    fs.mkdirSync(new URL('../.agent-debug/approval/',import.meta.url),{recursive:true});
    await client.send('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
    const desktop=await client.send('Page.captureScreenshot',{format:'png'});
    fs.writeFileSync(new URL('../.agent-debug/approval/approval-groups-desktop.png',import.meta.url),Buffer.from(desktop.data,'base64'));
    await client.send('Emulation.setDeviceMetricsOverride',{width:360,height:800,deviceScaleFactor:1,mobile:false});
    await run(`document.querySelector('[data-approval-group]').click()`);
    assert.equal(await run(`document.getElementById('approval-rule-groups-dialog').scrollWidth<=document.getElementById('approval-rule-groups-dialog').clientWidth`),true);
    const narrow=await client.send('Page.captureScreenshot',{format:'png'});
    fs.writeFileSync(new URL('../.agent-debug/approval/approval-groups-narrow.png',import.meta.url),Buffer.from(narrow.data,'base64'));
    assert.equal(await run(`__cotermTest.invocations.some(call=>['invoke_operation','approve_proposal'].includes(call.command))`),false);
  } finally {await session.close();}
});

test("instances share a group, offline bindings are editable, and group updates show every affected instance", async t => {
  const scenario=twoInstances();
  scenario.workspace={approvalRuleGroups:[{id:'shared',name:'Shared rules',mode:'allow_matches',patterns:['^pwd$']},{id:'other',name:'Other rules',mode:'approve_matches',patterns:['reboot']}],
    entries:[...scenario.instances.map(instance=>({configRef:'ref-'+instance.driverInstanceId,driverId:instance.driverId,label:instance.label,instanceId:instance.driverInstanceId})),
      {configRef:'offline',driverId:'ssh',label:'Offline board',instanceId:null}]};
  const session=await harness(t,scenario);if(!session)return;
  const {run,wait,click}=session;
  const assign=async id=>{await run(`document.getElementById('approval-group-select').value=${JSON.stringify(id)};document.getElementById('approval-group-select').dispatchEvent(new Event('change',{bubbles:true}))`);await click('Save assignment');};
  try {
    await wait(`!!document.querySelector('button[aria-label="Open approval settings"]')`);
    await run(`document.querySelector('button[aria-label="Open approval settings"]').click()`);await wait(`!!document.getElementById('approval-group-select')`);
    await run(`window.assignGate=__cotermTest.gate('assign_approval_group')`);await assign('shared');await wait(`assignGate.size()===1`);
    await run(`assignGate.fail('disk unavailable')`);await wait(`document.querySelector('[role=alert]')?.textContent.includes('disk unavailable')`);
    assert.equal(await run(`document.getElementById('approval-group-select').value`),'shared');
    assert.equal(await run(`__cotermTest.workspace().entries[0].approvalRuleGroupId`),null);
    await click('Save assignment');await wait(`__cotermTest.workspace().entries[0].approvalRuleGroupId==='shared'`);
    await run(`document.querySelector('#approval-settings-dialog button[aria-label="Close dialog"]').click();document.getElementById('instance-tab-inst-2').click()`);
    await run(`document.querySelector('button[aria-label="Open approval settings"]').click()`);await wait(`!!document.getElementById('approval-group-select')`);
    await assign('shared');await wait(`__cotermTest.workspace().entries[1].approvalRuleGroupId==='shared'`);
    await click('Manage rule groups');await wait(`!!document.getElementById('approval-group-name')`);
    assert.equal(await run(`document.querySelector('[data-group-users]').textContent.includes('Prod-Server') && document.querySelector('[data-group-users]').textContent.includes('Staging-Server')`),true);
    assert.equal(await run(`[...document.querySelectorAll('dialog button')].find(button=>button.textContent==='Delete group').disabled`),true);
    await run(`const input=document.querySelector('input[aria-label="Rule 1"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'^uname$');input.dispatchEvent(new Event('input',{bubbles:true}))`);
    await click('Save group');await wait(`__cotermTest.workspace().approvalRuleGroups.find(group=>group.id==='shared').patterns[0]==='^uname$'`);
    assert.deepEqual(await run(`__cotermTest.workspace().approvalRuleGroups.find(group=>group.id==='other').patterns`),['reboot']);
    await run(`document.querySelector('#approval-rule-groups-dialog button[aria-label="Close dialog"]').click()`);await wait(`!!document.getElementById('approval-group-select')`);
    await click('View rules');await wait(`document.querySelector('#approval-settings-dialog code')?.textContent==='^uname$'`);
    await run(`document.querySelector('#approval-settings-dialog button[aria-label="Close dialog"]').click();document.querySelector('button[aria-label="Configure approval for Offline board"]').click()`);
    await wait(`document.querySelector('[data-approval-instance]')?.textContent==='Offline board' && !!document.getElementById('approval-group-select')`);
    await assign('other');await wait(`__cotermTest.workspace().entries.find(entry=>entry.configRef==='offline').approvalRuleGroupId==='other'`);
    assert.equal(await run(`__cotermTest.countCalls('restore_saved_instance')`),0);
    await assign('');await wait(`__cotermTest.workspace().entries.find(entry=>entry.configRef==='offline').approvalRuleGroupId===null`);
    assert.equal(await run(`document.getElementById('approval-settings-dialog').textContent.includes('All Agent commits require approval.')`),true);
  } finally {await session.close();}
});

test("instance bypass writes and stale reads remain isolated from group configuration", async t => {
  const session=await harness(t,twoInstances());if(!session)return;
  const {client,run,wait}=session;
  try {
    await wait(`!!document.getElementById('instance-tab-inst-1')`);
    await run(`document.getElementById('instance-tab-inst-1').click();window.confirm=()=>true`);
    await wait(`document.querySelector('button[aria-label="Enable approval bypass"]')?.disabled===false`);
    await run(`window.failedBypassSave=__cotermTest.gate('set_bypass');document.querySelector('button[aria-label="Enable approval bypass"]').click()`);
    await wait(`failedBypassSave.size()===1`);
    await run(`failedBypassSave.fail('disk unavailable')`);
    await wait(`document.querySelector('button[aria-label="Enable approval bypass"]')?.disabled===false`);
    await run(`document.querySelector('button[aria-label="Enable approval bypass"]').click()`);
    await wait(`document.querySelector('button[aria-label="Disable approval bypass"]')?.getAttribute('aria-pressed')==='true'`);
    await run(`document.getElementById('instance-tab-inst-2').click()`);
    await wait(`document.querySelector('button[aria-label="Enable approval bypass"]')?.disabled===false`);
    await run(`document.getElementById('instance-tab-inst-1').click()`);
    await wait(`document.querySelector('button[aria-label="Disable approval bypass"]')?.disabled===false`);
    assert.equal(await run(`__cotermTest.invocations.find(call=>call.command==='set_bypass').args.driverInstanceId`), "inst-1");
    await run(`window.bypassReadGate=__cotermTest.gate('get_bypass');document.getElementById('instance-tab-inst-2').click()`);
    await wait(`bypassReadGate.size()===1`);
    await run(`document.getElementById('instance-tab-inst-1').click()`);
    await wait(`bypassReadGate.size()===2`);
    await run(`bypassReadGate.release(true,1)`);
    await wait(`document.querySelector('button[aria-label="Disable approval bypass"]')?.disabled===false`);
    await run(`bypassReadGate.release(false);new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
    assert.equal(await run(`document.querySelector('button[aria-label="Disable approval bypass"]')?.disabled`), false, "late reads for another instance cannot replace the current instance state");
    await run(`window.bypassWriteGate=__cotermTest.gate('set_bypass');document.querySelector('button[aria-label="Disable approval bypass"]').click()`);
    await wait(`bypassWriteGate.size()===1`);
    await run(`document.getElementById('instance-tab-inst-2').click()`);
    await wait(`__cotermTest.invocations.filter(call=>call.command==='get_bypass').at(-1)?.args.driverInstanceId==='inst-2'`);
    await run(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));bypassWriteGate.release(null)`);
    await wait(`document.querySelector('button[aria-label="Enable approval bypass"]')?.disabled===false`);
    await run(`document.getElementById('instance-tab-inst-1').click()`);
    await wait(`document.querySelector('button[aria-label="Disable approval bypass"]')?.disabled===false`);

    await run(`document.querySelector('button[aria-label="Open approval settings"]').click()`);
    await wait(`document.getElementById('approval-settings-dialog')?.textContent.includes('Bypass is on')`);
    await setAppLocale(client,'zh-CN');await wait(`document.getElementById('approval-settings-dialog').textContent.includes('Bypass 已开启')`);
  } finally {await session.close();}
});
