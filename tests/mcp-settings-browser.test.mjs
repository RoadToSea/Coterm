/** Settings and the Stdio configuration dialog use the real host API; only IPC is mocked. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { assertFreshBuild, evaluate, findBrowser, openSession, rootDir, startStaticServer, waitForValue, setAppLocale } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("MCP settings validate, save ports, preserve failures and publish matching client arguments", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshBuild();
  const scenario = structuredClone(defaultScenario);
  scenario.mcp = {enabled:true,port:51234,occupiedPorts:[51235]};
  const server = await startStaticServer();
  const session = await openSession({browserPath});
  const {client} = session;
  const run = source => evaluate(client,source);
  const wait = source => waitForValue(client,source);
  const input = async (id,value) => run(`(() => {
    const input=document.getElementById(${JSON.stringify(id)});
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,${JSON.stringify(String(value))});
    input.dispatchEvent(new Event('input',{bubbles:true}));
  })()`);
  const submit = () => run(`document.getElementById('mcp-port').form.requestSubmit()`);
  const portCalls = () => run(`__cotermTest.invocations.filter(c=>c.command==='set_mcp_stdio_port')`);
  const openConnection = async () => {
    await run(`document.querySelector('[data-setting=mcp-connection] [data-mcp-config]').click()`);
    await wait(`!!document.querySelector('dialog[aria-label="MCP connection"]')?.open`);
  };
  const closeConnection = async () => {
    await run(`document.querySelector('dialog[aria-label="MCP connection"] header button').click()`);
    await wait(`!document.querySelector('dialog[aria-label="MCP connection"]')`);
  };
  const shot = async name => {
    const directory=process.env.COTERM_MCP_SETTINGS_SCREENSHOT_DIR;
    if(!directory)return;
    fs.mkdirSync(directory,{recursive:true});
    const result=await client.send("Page.captureScreenshot",{format:"png"});
    fs.writeFileSync(path.join(directory,name+'.png'),Buffer.from(result.data,'base64'));
  };
  const errors=[];
  try {
    await client.send('Page.enable'); await client.send('Runtime.enable');
    client.on('Runtime.exceptionThrown',event=>errors.push(event.exceptionDetails.exception?.description??event.exceptionDetails.text));
    await client.send('Emulation.setDeviceMetricsOverride',{width:1100,height:800,deviceScaleFactor:1,mobile:false});
    await client.send('Page.addScriptToEvaluateOnNewDocument',{source:makeHostMockSource(scenario)});
    await client.send('Page.navigate',{url:server.url});
    await wait(`!!document.getElementById('open-settings')`);
    await run(`document.getElementById('open-settings').click()`);
    await wait(`document.getElementById('mcp-port')?.value==='51234'`);
    await run(`document.querySelector('[data-category=terminal]').click()`);
    await input('settings-search','端口');
    await wait(`!!document.getElementById('mcp-port')`);
    assert.equal(await run(`document.querySelectorAll('[data-setting]').length`),1,'search works across categories and languages');
    await run(`document.querySelector('[data-category=mcp]').click()`);
    await wait(`document.getElementById('mcp-port')?.value==='51234'`);
    const beforeProtocol=await run(`__cotermTest.invocations.filter(c=>c.command==='invoke_operation' && ['ssh.connect','ssh.disconnect','ssh.write'].includes(c.args.operation)).length`);
    for(const invalid of ['0','65536','1.5','']){
      await input('mcp-port',invalid); await submit();
      await wait(`document.getElementById('mcp-settings-error')?.textContent.includes('between 1 and 65535')`);
      assert.equal(await run(`document.getElementById('mcp-port').getAttribute('aria-invalid')`),'true');
    }
    assert.equal((await portCalls()).length,0);
    await input('mcp-port',51235); await submit();
    await wait(`document.getElementById('mcp-settings-error')?.textContent.includes('already in use')`);
    assert.equal(await run(`document.getElementById('mcp-port').value`),'51235');
    assert.equal(await run(`__cotermTest.mcpStatus().port`),51234);

    await run(`window.saveGate=__cotermTest.gate('set_mcp_stdio_port')`);
    await input('mcp-port',51236); await submit();
    await wait(`window.saveGate.size()===1`);
    assert.equal(await run(`document.getElementById('mcp-port').disabled`),true);
    await submit(); await submit();
    assert.equal(await run(`window.saveGate.size()`),1,'only one save can be in flight');
    await run(`window.saveGate.fail('mcp_settings_write_failed')`);
    await wait(`document.getElementById('mcp-settings-error')?.textContent.includes('Could not save')`);
    assert.equal(await run(`document.getElementById('mcp-port').value`),'51236');
    assert.equal(await run(`__cotermTest.mcpStatus().port`),51234);

    await input('mcp-port',51237); await submit();
    await wait(`document.getElementById('mcp-port').form.textContent.includes('Saved')`);
    assert.deepEqual((await portCalls()).at(-1).args,{request:{port:51237}});
    assert.equal(await run(`__cotermTest.mcpStatus().address`),'127.0.0.1:51237');
    await shot('mcp-settings-light');
    await openConnection();
    await wait(`!!document.querySelector('textarea[aria-label="Stdio client configuration"]')`);
    assert.deepEqual(await run(`JSON.parse(document.querySelector('textarea[aria-label="Stdio client configuration"]').value).mcpServers.coterm.args`),['mcp','stdio','--addr','127.0.0.1:51237']);
    await closeConnection();
    await run(`document.querySelector('#mcp-connection button[data-state=off]').click()`);
    await wait(`!!document.getElementById('mcp-toggle')`);
    await run(`document.getElementById('mcp-toggle').click()`);
    await wait(`document.querySelector('[data-mcp-status]').textContent.includes('Not started')`);
    const enables=await run(`__cotermTest.calls.filter(c=>c==='set_mcp_stdio_enabled').length`);
    await input('mcp-port',51238); await submit();
    await wait(`document.getElementById('mcp-port').form.textContent.includes('Saved')`);
    assert.equal(await run(`__cotermTest.mcpStatus().enabled`),false);
    assert.equal(await run(`__cotermTest.calls.filter(c=>c==='set_mcp_stdio_enabled').length`),enables);
    assert.equal(await run(`__cotermTest.invocations.filter(c=>c.command==='invoke_operation' && ['ssh.connect','ssh.disconnect','ssh.write'].includes(c.args.operation)).length`),beforeProtocol);

    await run(`document.querySelector('[data-category=appearance]').click()`);
    await wait(`!!document.querySelector('#settings-theme-switcher button[aria-label="Dark mode"]')`);
    await run(`document.querySelector('#settings-theme-switcher button[aria-label="Dark mode"]').click()`);
    await run(`document.querySelector('[data-category=mcp]').click()`);
    await wait(`document.getElementById('mcp-port')?.value==='51238'`);
    await shot('mcp-settings-dark');
    await setAppLocale(client,'zh-CN');
    await run(`document.querySelector('[data-category=mcp]').click()`);
    await wait(`document.getElementById('mcp-port')?.value==='51238'`);
    assert.equal(await run(`document.querySelector('label[for=mcp-port]').textContent`),'MCP 端口');
    await run(`document.querySelector('.settings-footer button').click()`);
    assert.equal(await run(`__cotermTest.mcpStatus().port`),51238,'font reset never changes MCP settings');
    await client.send('Emulation.setDeviceMetricsOverride',{width:360,height:720,deviceScaleFactor:1,mobile:false});
    assert.equal(await run(`document.getElementById('app-settings-dialog').scrollWidth<=document.getElementById('app-settings-dialog').clientWidth`),true);
    assert.equal(await run(`document.getElementById('mcp-port').getBoundingClientRect().right<=360`),true);
    await shot('mcp-settings-zh-narrow');
    await client.send('Page.reload');
    await wait(`!!document.getElementById('open-settings')`);
    await run(`document.getElementById('open-settings').click()`);
    await wait(`!!document.querySelector('[data-category=mcp]')`);
    await run(`document.querySelector('[data-category=mcp]').click()`);
    await wait(`document.getElementById('mcp-port')?.value==='51238'`);
    assert.deepEqual(errors,[]);
  } finally {await session.close(); await new Promise(resolve=>server.server.close(resolve));}
});

test("MCP settings recover read failures and damaged preferences without a late update after closing", async t => {
  const browserPath=findBrowser(); if(!browserPath){t.skip('No Chrome/Edge binary');return;}
  assertFreshBuild();
  const scenario=structuredClone(defaultScenario);
  scenario.mcp={enabled:false,configurationError:'mcp_settings_invalid'};
  scenario.failures={mcp_stdio_status:{sequence:[1]}};
  const server=await startStaticServer(),session=await openSession({browserPath});
  const {client}=session, run=source=>evaluate(client,source), wait=source=>waitForValue(client,source);
  try{
    await client.send('Page.enable');await client.send('Runtime.enable');
    await client.send('Page.addScriptToEvaluateOnNewDocument',{source:makeHostMockSource(scenario)});
    await client.send('Page.navigate',{url:server.url});
    await wait(`!!document.getElementById('open-settings')`);
    await run(`document.getElementById('open-settings').click()`);
    await wait(`!!document.getElementById('mcp-settings-error')`);
    assert.equal(await run(`document.getElementById('mcp-port').disabled`),true);
    await run(`document.querySelector('#mcp-settings-error button').click()`);
    await wait(`document.getElementById('mcp-settings-error').textContent.includes('saved MCP settings are invalid')`);
    assert.equal(await run(`document.getElementById('mcp-port').form.querySelector('[type=submit]').disabled`),false,'saving even the displayed default recovers a damaged file');
    await run(`document.getElementById('mcp-port').form.requestSubmit()`);
    await wait(`!document.getElementById('mcp-settings-error') && __cotermTest.mcpStatus().configurationError===null`);
    assert.equal(await run(`__cotermTest.mcpStatus().enabled`),false);
    await run(`document.querySelector('#app-settings-dialog header button').click()`);
    await wait(`!document.getElementById('app-settings-dialog')`);
    await run(`window.loadGate=__cotermTest.gate('mcp_stdio_status');document.getElementById('open-settings').click()`);
    await wait(`window.loadGate.size()>=1`);
    await run(`document.querySelector('#app-settings-dialog header button').click();window.loadGate.release(__cotermTest.mcpStatus());__cotermTest.clearGates()`);
    assert.equal(await run(`!!document.getElementById('app-settings-dialog')`),false);
  }finally{await session.close();await new Promise(resolve=>server.server.close(resolve));}
});


test("MCP connection has equal capsule options, manual controls, live status and persistent startup policy", async t => {
  const browserPath=findBrowser(); if(!browserPath){t.skip('No Chrome/Edge binary');return;}
  assertFreshBuild();
  const scenario=structuredClone(defaultScenario);
  scenario.mcp={startupMode:'manual',enabled:false};
  const server=await startStaticServer(),session=await openSession({browserPath});
  const {client}=session,run=source=>evaluate(client,source),wait=source=>waitForValue(client,source);
  const mode=async value=>{await run(`document.querySelectorAll('#mcp-connection button')[${value==='auto'?0:1}].click()`);};
  const open=async()=>{
    await wait(`!!document.getElementById('open-settings')`);
    await run(`document.getElementById('open-settings').click()`);
    await wait(`!!document.querySelector('[data-category=mcp]')`);
    await run(`document.querySelector('[data-category=mcp]').click()`);
    await wait(`!!document.querySelector('#mcp-connection button[data-state=on]')`);
  };
  const errors=[];
  try{
    await client.send('Page.enable');await client.send('Runtime.enable');
    client.on('Runtime.exceptionThrown',event=>errors.push(event.exceptionDetails.exception?.description??event.exceptionDetails.text));
    await client.send('Page.addScriptToEvaluateOnNewDocument',{source:makeHostMockSource(scenario)});
    await client.send('Emulation.setDeviceMetricsOverride',{width:1100,height:800,deviceScaleFactor:1,mobile:false});
    await client.send('Page.navigate',{url:server.url});await open();
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Not started');
    assert.equal(await run(`document.querySelectorAll('[data-setting]').length`),2);
    assert.equal(await run(`document.querySelector('[data-setting=mcp-port]').innerText`),'MCP port\nSave');
    assert.equal(await run(`document.getElementById('mcp-toggle').textContent`),'Start MCP');
    assert.equal(await run(`(()=>{const items=[...document.querySelectorAll('#mcp-connection button')];return items.length===2&&Math.abs(items[0].getBoundingClientRect().width-items[1].getBoundingClientRect().width)<1&&items.every(item=>parseFloat(getComputedStyle(item).borderRadius)>=20);})()`),true);
    const calls=await run(`__cotermTest.calls.filter(c=>c==='set_mcp_startup_mode').length`);
    await mode('manual');
    assert.equal(await run(`__cotermTest.calls.filter(c=>c==='set_mcp_startup_mode').length`),calls,'selected option cannot be deselected');

    await run(`window.startGate=__cotermTest.gate('set_mcp_stdio_enabled');document.getElementById('mcp-toggle').click()`);
    await wait(`startGate.size()===1`);
    assert.equal(await run(`document.getElementById('mcp-toggle').disabled&&document.getElementById('mcp-port').disabled&&[...document.querySelectorAll('#mcp-connection button')].every(b=>b.disabled)`),true);
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Not started','never reports success before IPC completes');
    await run(`startGate.fail('mcp_port_unavailable')`);
    await wait(`document.getElementById('mcp-settings-error')?.textContent.includes('already in use')`);
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Not started');
    await run(`document.getElementById('mcp-toggle').click()`);
    await wait(`document.querySelector('[data-mcp-status] [role=status]').textContent==='Started'`);
    assert.equal(await run(`document.getElementById('mcp-toggle').textContent`),'Stop MCP');
    await run(`document.getElementById('mcp-toggle').click()`);
    await wait(`document.querySelector('[data-mcp-status] [role=status]').textContent==='Not started'`);

    await run(`window.modeGate=__cotermTest.gate('set_mcp_startup_mode')`);await mode('auto');
    await wait(`modeGate.size()===1`);
    await run(`modeGate.fail('mcp_settings_write_failed')`);
    await wait(`document.getElementById('mcp-settings-error')?.textContent.includes('Could not save')`);
    assert.equal(await run(`document.querySelector('#mcp-connection button[data-state=on]').textContent`),'Manual start');
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Not started');
    await mode('auto');
    await wait(`!document.getElementById('mcp-toggle')&&document.querySelector('[data-mcp-status] [role=status]').textContent==='Started'`);
    assert.deepEqual(await run(`__cotermTest.invocations.filter(c=>c.command==='set_mcp_startup_mode').at(-1).args`),{request:{startupMode:'auto'}});
    await client.send('Page.reload');await open();
    assert.equal(await run(`document.querySelector('#mcp-connection button[data-state=on]').textContent`),'Auto start');
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Started');
    await mode('manual');await wait(`!!document.getElementById('mcp-toggle')`);
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Started','manual policy does not interrupt an active client');
    await client.send('Page.reload');await open();
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Not started');
    await run(`document.getElementById('mcp-toggle').click()`);
    await wait(`document.querySelector('[data-mcp-status] [role=status]').textContent==='Started'`);
    await client.send('Page.reload');await open();
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'Not started','manual start is not restored as automatic startup');
    await setAppLocale(client,'zh-CN');
    await run(`document.querySelector('[data-category=mcp]').click()`);
    await wait(`document.querySelector('#mcp-connection button[data-state=on]')?.textContent==='手动启动'`);
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'未启动');
    assert.equal(await run(`document.getElementById('mcp-toggle').textContent`),'启动 MCP');
    await run(`document.querySelector('[data-category=general]').click()`);
    assert.equal(await run(`document.querySelector('[data-mcp-status] [role=status]').textContent`),'未启动','status remains at the top of every category');
    assert.deepEqual(errors,[]);
  }finally{await session.close();await new Promise(resolve=>server.server.close(resolve));}
});
