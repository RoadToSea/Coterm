/** Real host and Serial configuration; a stopped backend must start before enumeration. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { assertFreshBuild, findBrowser, openSession, evaluate, waitForValue, startStaticServer, setAppLocale } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("Serial creation lists COM6 from a stopped plugin before any instance is submitted", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshBuild();
  assert.ok(fs.statSync(new URL("../plugins/plugin-serial/dist/frontend/index.js", import.meta.url)).mtimeMs >=
    fs.statSync(new URL("../plugins/plugin-serial/frontend/src/index.ts", import.meta.url)).mtimeMs, "build Serial first");
  const scenario = structuredClone(defaultScenario);
  scenario.catalog = {plugins:[{id:"serial",name:"Serial",version:"0.1.0",hasFrontend:true,running:false}],issues:[]};
  scenario.instances = [];
  scenario.serialPorts = [{portName:"COM6",portType:{kind:"usb",product:"USB-SERIAL CH340 (COM6)"}}];
  const server = await startStaticServer();
  const session = await openSession({browserPath});
  const {client} = session;
  const run = source => evaluate(client, source);
  const wait = expression => waitForValue(client, expression, {label:"Serial configuration enumeration"});
  const screenshot = async name => {
    const directory = process.env.COTERM_SERIAL_CONFIG_SCREENSHOT_DIR;
    if (!directory) return;
    fs.mkdirSync(directory, {recursive:true});
    const shot = await client.send("Page.captureScreenshot", {format:"png"});
    fs.writeFileSync(path.join(directory, name + ".png"), Buffer.from(shot.data, "base64"));
  };
  try {
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Page.addScriptToEvaluateOnNewDocument", {source:makeHostMockSource(scenario)});
    await client.send("Page.navigate", {url:server.url});
    await wait(`!!document.querySelector('button[aria-label="Manage Serial"]')`);
    await run(`window.configuration=()=>[...document.querySelectorAll('#plugin-configuration div')].find(el=>el.shadowRoot?.querySelector('form'))?.shadowRoot;
      document.querySelector('button[aria-label="Manage Serial"]').click()`);
    await wait(`!!configuration()?.querySelector('select') && !configuration().querySelector('select').textContent.includes('Loading')`);
    assert.deepEqual(await run(`[...configuration().querySelector('select').options].map(option=>option.value)`), ["COM6"],
      "a present system port must be selectable before creating an instance");
    const calls = await run(`__cotermTest.invocations.map(call=>call.command)`);
    assert.ok(calls.indexOf("start_plugin") >= 0 && calls.indexOf("start_plugin") < calls.indexOf("invoke_plugin_operation"),
      "the host must start the backend before requesting plugin-level data");
    assert.equal(calls.includes("create_instance"), false, "enumeration does not create or open a serial connection");
    await screenshot("serial-creation-com6");

    // A failed operation is not an empty device list, and retry must be possible.
    await run(`window.lookup=__cotermTest.gate('invoke_plugin_operation');configuration().querySelector('.refresh-ports').click()`);
    await wait(`lookup.size()===1`);
    assert.equal(await run(`configuration().querySelector('button[type=submit]').disabled`), true);
    assert.equal(await run(`configuration().querySelector('.refresh-ports').disabled`), true);
    await run(`lookup.release({status:'failed',output:null,error:{message:'device enumeration failed'}})`);
    await wait(`configuration().querySelector('.error').textContent.includes('device enumeration failed')`);
    assert.equal(await run(`configuration().querySelector('select').textContent.includes('No serial ports detected')`), false);
    assert.equal(await run(`configuration().querySelector('button[type=submit]').disabled`), true);

    await run(`window.lookup=__cotermTest.gate('invoke_plugin_operation');configuration().querySelector('.refresh-ports').click()`);
    await wait(`lookup.size()===1`);
    await setAppLocale(client, "zh-CN");
    assert.equal(await run(`configuration().querySelector('select').textContent`), "正在加载端口…",
      "locale changes preserve the loading state instead of claiming there are no ports");
    await run(`lookup.release({status:'completed',output:{ports:[]}})`);
    await wait(`configuration().querySelector('select').textContent==='未检测到串口'`);
    assert.equal(await run(`configuration().querySelector('.error').textContent`), "");
    assert.equal(await run(`configuration().querySelector('.refresh-ports').getAttribute('aria-label')`), "刷新端口");

    // A legacy backend missing the field must ask for an update, not report no devices.
    await run(`window.lookup=__cotermTest.gate('invoke_plugin_operation');configuration().querySelector('.refresh-ports').click()`);
    await wait(`lookup.size()===1`);
    await run(`lookup.release({status:'completed',output:{instanceCount:0}})`);
    await wait(`configuration().querySelector('.error').textContent.includes('请重新构建插件')`);

    await run(`window.lookup=__cotermTest.gate('invoke_plugin_operation');configuration().querySelector('.refresh-ports').click()`);
    await wait(`lookup.size()===1`);
    await run(`lookup.release({status:'completed',output:{ports:[{portName:'COM6',portType:{kind:'usb'}},{portName:'COM9',portType:{kind:'unknown'}}]}})`);
    await wait(`configuration().querySelector('select').options.length===2`);
    await run(`const port=configuration().querySelector('select');port.value='COM9';port.dispatchEvent(new Event('change',{bubbles:true}));
      window.lookup=__cotermTest.gate('invoke_plugin_operation');configuration().querySelector('.refresh-ports').click()`);
    await wait(`lookup.size()===1`);
    await run(`lookup.release({status:'completed',output:{ports:[{portName:'COM9',portType:{kind:'unknown'}},{portName:'COM6',portType:{kind:'usb'}}]}})`);
    await wait(`!configuration().querySelector('.refresh-ports').disabled`);
    assert.equal(await run(`configuration().querySelector('select').value`), "COM9", "refresh preserves the selected port if still present");
    assert.equal(await run(`configuration().querySelector('button[type=submit]').disabled`), false);
    assert.equal(await run(`__cotermTest.countCalls('create_instance')`), 0);

    await client.send("Emulation.setDeviceMetricsOverride", {width:360,height:720,deviceScaleFactor:1,mobile:false});
    await wait(`document.documentElement.scrollWidth<=innerWidth`);
    assert.ok(await run(`configuration().querySelector('select').clientWidth>=100`), "the detected port name remains readable in a narrow window");
    await screenshot("serial-creation-narrow");
  } finally { await session.close(); await new Promise(resolve => server.server.close(resolve)); }
});

test("Serial discovery exposes startup failure and cancels lookup when its configuration is closed", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshBuild();
  const scenario = structuredClone(defaultScenario);
  scenario.catalog = {plugins:[
    {id:"serial",name:"Serial",version:"0.1.0",hasFrontend:true,running:false},
    {id:"local",name:"Local",version:"0.1.0",hasFrontend:false,running:false},
  ],issues:[]};
  scenario.instances = [];
  const server = await startStaticServer();
  const session = await openSession({browserPath});
  const {client} = session;
  const run = source => evaluate(client, source);
  const wait = expression => waitForValue(client, expression, {label:"Serial startup failure and unmount"});
  try {
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Page.addScriptToEvaluateOnNewDocument", {source:makeHostMockSource(scenario)});
    await client.send("Page.navigate", {url:server.url});
    await wait(`!!document.querySelector('button[aria-label="Manage Serial"]')`);
    await run(`window.configuration=()=>[...document.querySelectorAll('#plugin-configuration div')].find(el=>el.shadowRoot?.querySelector('form'))?.shadowRoot;
      window.starting=__cotermTest.gate('start_plugin');document.querySelector('button[aria-label="Manage Serial"]').click()`);
    await wait(`starting.size()===1`);
    assert.equal(await run(`__cotermTest.countCalls('invoke_plugin_operation')`), 0, "enumeration waits for the startup handshake");
    await run(`starting.fail('backend startup failed')`);
    await wait(`configuration()?.querySelector('.error').textContent.includes('backend startup failed')`);
    assert.equal(await run(`configuration().querySelector('select').textContent`), "Could not load serial ports");
    await run(`window.starting=__cotermTest.gate('start_plugin');configuration().querySelector('.refresh-ports').click()`);
    await wait(`starting.size()===1`);
    await run(`document.querySelector('button[aria-label="Manage Local"]').click()`);
    await wait(`document.getElementById('plugin-configuration').textContent.includes('does not provide a connection configuration screen')`);
    await run(`starting.release(null);new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
    assert.equal(await run(`__cotermTest.countCalls('invoke_plugin_operation')`), 0, "a closed configuration never sends a late lookup");
    assert.equal(await run(`__cotermTest.countCalls('create_instance')`), 0);
  } finally { await session.close(); await new Promise(resolve => server.server.close(resolve)); }
});
