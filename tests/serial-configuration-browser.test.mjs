/** Real host and Serial configuration; a stopped backend must start before enumeration. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { assertFreshBuild, findBrowser, openSession, evaluate, waitForValue, startStaticServer, setAppLocale } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("Serial creation lists COM6 from a stopped driver before any instance is submitted", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshBuild();
  assert.ok(fs.statSync(new URL("../drivers/driver-serial/dist/frontend/index.js", import.meta.url)).mtimeMs >=
    fs.statSync(new URL("../drivers/driver-serial/frontend/src/index.ts", import.meta.url)).mtimeMs, "build Serial first");
  const scenario = structuredClone(defaultScenario);
  scenario.catalog = {drivers:[{id:"serial",name:"Serial",version:"0.1.0",hasFrontend:true,running:false}],issues:[]};
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
    await run(`window.configuration=()=>[...document.querySelectorAll('#driver-configuration div')].find(el=>el.shadowRoot?.querySelector('form'))?.shadowRoot;
      document.querySelector('button[aria-label="Manage Serial"]').click()`);
    await wait(`!!configuration()?.querySelector('select') && !configuration().querySelector('select').textContent.includes('Loading')`);
    assert.deepEqual(await run(`[...configuration().querySelector('select').options].map(option=>option.value)`), ["COM6"],
      "a present system port must be selectable before creating an instance");
    const calls = await run(`__cotermTest.invocations.map(call=>call.command)`);
    assert.ok(calls.indexOf("start_driver") >= 0 && calls.indexOf("start_driver") < calls.indexOf("invoke_driver_operation"),
      "the host must start the backend before requesting driver-level data");
    assert.equal(calls.includes("create_instance"), false, "enumeration does not create or open a serial connection");
    await screenshot("serial-creation-com6");

    // Exercise an actual mouse opening, not just a scripted click on the element.
    await run(`window.lookup=__cotermTest.gate('invoke_driver_operation');window.beforeOpen=__cotermTest.countCalls('invoke_driver_operation')`);
    const point = await run(`(()=>{const r=configuration().querySelector('select').getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    await client.send("Input.dispatchMouseEvent", {type:"mousePressed",...point,button:"left",clickCount:1});
    await client.send("Input.dispatchMouseEvent", {type:"mouseReleased",...point,button:"left",clickCount:1});
    await wait(`lookup.size()===1`);
    assert.equal(await run(`configuration().querySelector('select').matches(':open')`), false, "read new ports before the native picker snapshots its options");
    await run(`lookup.release({status:'completed',output:{ports:[{portName:'COM6',portType:{kind:'usb'}},{portName:'COM8',portType:{kind:'unknown'}}]}})`);
    await wait(`configuration().querySelector('option[value="COM8"]')`);
    await wait(`configuration().querySelector('select').matches(':open')`);
    for (const [key,code,windowsVirtualKeyCode] of [["End","End",35],["Enter","Enter",13]]) {
      await client.send("Input.dispatchKeyEvent", {type:"keyDown",key,code,windowsVirtualKeyCode});
      await client.send("Input.dispatchKeyEvent", {type:"keyUp",key,code,windowsVirtualKeyCode});
    }
    await wait(`configuration().querySelector('select').value==='COM8'`);
    assert.equal(await run(`__cotermTest.countCalls('invoke_driver_operation')`), await run(`beforeOpen+1`), "selecting an option does not trigger a second lookup");

    // A failed operation is not an empty device list, and retry must be possible.
    await run(`window.lookup=__cotermTest.gate('invoke_driver_operation');configuration().querySelector('select').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}))`);
    await wait(`lookup.size()===1`);
    assert.equal(await run(`configuration().querySelector('button[type=submit]').disabled`), true);
    assert.equal(await run(`configuration().querySelector('select').disabled`), false, 'a pending lookup does not block the native picker');
    await run(`lookup.release({status:'failed',output:null,error:{message:'device enumeration failed'}})`);
    await wait(`configuration().querySelector('.error').textContent.includes('device enumeration failed')`);
    assert.equal(await run(`configuration().querySelector('select').textContent.includes('No serial ports detected')`), false);
    assert.equal(await run(`configuration().querySelector('button[type=submit]').disabled`), true);

    await run(`window.lookup=__cotermTest.gate('invoke_driver_operation');configuration().querySelector('select').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}))`);
    await wait(`lookup.size()===1`);
    await setAppLocale(client, "zh-CN");
    assert.equal(await run(`configuration().querySelector('select').textContent`), "正在加载端口…",
      "locale changes preserve the loading state instead of claiming there are no ports");
    await run(`lookup.release({status:'completed',output:{ports:[]}})`);
    await wait(`configuration().querySelector('select').textContent==='未检测到串口'`);
    assert.equal(await run(`configuration().querySelector('.error').textContent`), "");
    assert.equal(await run(`configuration().querySelector('.refresh-ports')`), null, 'refresh has no separate button');

    // A legacy backend missing the field must ask for an update, not report no devices.
    await run(`window.lookup=__cotermTest.gate('invoke_driver_operation');configuration().querySelector('select').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}))`);
    await wait(`lookup.size()===1`);
    await run(`lookup.release({status:'completed',output:{instanceCount:0}})`);
    await wait(`configuration().querySelector('.error').textContent.includes('请重新构建驱动')`);

    await run(`window.lookup=__cotermTest.gate('invoke_driver_operation');configuration().querySelector('select').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}))`);
    await wait(`lookup.size()===1`);
    await run(`lookup.release({status:'completed',output:{ports:[{portName:'COM6',portType:{kind:'usb'}},{portName:'COM9',portType:{kind:'unknown'}}]}})`);
    await wait(`configuration().querySelector('select').options.length===2`);
    await run(`const port=configuration().querySelector('select');port.value='COM9';port.dispatchEvent(new Event('change',{bubbles:true}));
      window.lookup=__cotermTest.gate('invoke_driver_operation');configuration().querySelector('select').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}))`);
    await wait(`lookup.size()===1`);
    await run(`lookup.release({status:'completed',output:{ports:[{portName:'COM9',portType:{kind:'unknown'}},{portName:'COM6',portType:{kind:'usb'}}]}})`);
    await wait(`configuration().querySelector('form').getAttribute('aria-busy')==='false'`);
    assert.equal(await run(`configuration().querySelector('select').value`), "COM9", "refresh preserves the selected port if still present");
    assert.equal(await run(`configuration().querySelector('button[type=submit]').disabled`), false);
    assert.equal(await run(`__cotermTest.countCalls('create_instance')`), 0);

    // Keyboard openings also refresh once; held keys and repeated pointer presses are coalesced.
    await run(`(()=>{window.lookup=__cotermTest.gate('invoke_driver_operation');window.beforeKeyboard=__cotermTest.countCalls('invoke_driver_operation');
      const port=configuration().querySelector('select');port.dispatchEvent(new KeyboardEvent('keydown',{key:' ',bubbles:true}));
      port.dispatchEvent(new KeyboardEvent('keydown',{key:' ',repeat:true,bubbles:true}));
      port.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}));})()`);
    await wait(`lookup.size()===1`);
    assert.equal(await run(`__cotermTest.countCalls('invoke_driver_operation')`), await run(`beforeKeyboard+1`));
    await run(`lookup.release({status:'completed',output:{ports:[{portName:'COM9',portType:{kind:'unknown'}}]}})`);
    await wait(`configuration().querySelector('form').getAttribute('aria-busy')==='false'`);

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
  scenario.catalog = {drivers:[
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
    await run(`window.configuration=()=>[...document.querySelectorAll('#driver-configuration div')].find(el=>el.shadowRoot?.querySelector('form'))?.shadowRoot;
      window.starting=__cotermTest.gate('start_driver');document.querySelector('button[aria-label="Manage Serial"]').click()`);
    await wait(`starting.size()===1`);
    assert.equal(await run(`__cotermTest.countCalls('invoke_driver_operation')`), 0, "enumeration waits for the startup handshake");
    await run(`starting.fail('backend startup failed')`);
    await wait(`configuration()?.querySelector('.error').textContent.includes('backend startup failed')`);
    assert.equal(await run(`configuration().querySelector('select').textContent`), "Could not load serial ports");
    await run(`window.starting=__cotermTest.gate('start_driver');configuration().querySelector('select').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}))`);
    await wait(`starting.size()===1`);
    await run(`document.querySelector('button[aria-label="Manage Local"]').click()`);
    await wait(`document.getElementById('driver-configuration').textContent.includes('does not provide a connection configuration screen')`);
    await run(`starting.release(null);new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
    assert.equal(await run(`__cotermTest.countCalls('invoke_driver_operation')`), 0, "a closed configuration never sends a late lookup");
    assert.equal(await run(`__cotermTest.countCalls('create_instance')`), 0);
  } finally { await session.close(); await new Promise(resolve => server.server.close(resolve)); }
});
