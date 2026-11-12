import { assertFreshDriverBuild } from "./support/browser-harness.mjs";
/** Real Serial bundle and xterm; no physical port is opened. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("receive newline settings update xterm, preserve raw bytes and persist per instance", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshDriverBuild("serial");
  const artifact = new URL("../crates/coterm-driver/dist/serial/frontend/index.js", import.meta.url);
  for (const file of ["index.ts"]) {
    assert.ok(fs.statSync(artifact).mtimeMs >= fs.statSync(new URL(`../crates/coterm-driver/frontend/serial/src/${file}`, import.meta.url)).mtimeMs,
      "build the Serial frontend before testing");
  }
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const server = http.createServer((_request, response) => response.end("<!doctype html><html><body></body></html>"));
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const session = await openSession({ browserPath });
  const { client } = session;
  const run = source => evaluate(client, source);
  const rows = () => run(`(async () => {
    await new Promise(requestAnimationFrame);
    return Array.from(root.querySelectorAll('.xterm-rows > div'), row => row.textContent.replace(/\\u00a0/g,' ').trimEnd());
  })()`);
  const change = async mode => run(`(() => {
    root.querySelector('.settings-toggle').click();
    const lf = root.querySelector('[name=receiveCompleteLF]'), cr = root.querySelector('[name=receiveCompleteCR]');
    lf.checked = ${mode === "lf" || mode === "both"}; cr.checked = ${mode === "cr" || mode === "both"};
    lf.dispatchEvent(new Event('change', {bubbles:true}));
    root.querySelector('.settings-toggle').click();
  })()`);
  const checks = () => run(`Array.from(root.querySelectorAll('.receive-newlines input'), input => input.checked)`);
  const emit = text => run(`window.emit(${JSON.stringify(text)})`);
  const clear = () => run(`root.querySelector('[data-control=clear] button').click(); window.rawOutput='';`);
  const screenshot = async name => {
    const directory = process.env.COTERM_SERIAL_NEWLINES_SCREENSHOT_DIR;
    if (!directory) return;
    fs.mkdirSync(directory, { recursive: true });
    const shot = await client.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(path.join(directory, name + ".png"), Buffer.from(shot.data, "base64"));
  };
  try {
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Emulation.setDeviceMetricsOverride", {width:1000,height:760,deviceScaleFactor:1,mobile:false});
    await client.send("Page.navigate", {url:`http://127.0.0.1:${server.address().port}/`});
    await waitForValue(client, `document.readyState === 'complete'`);
    await run(`(async () => {
      document.body.style.cssText='margin:0;width:1000px;height:760px';
      window.calls=[]; window.cursor=0; window.rawOutput=''; window.rx=0; window.connection='conn';
      window.locale='zh-CN';
      window.driverModule=await import(${JSON.stringify(moduleURL)});
      window.receipt=output => ({operationId:'read',status:'completed',output,error:null,proposalId:null,waitExpired:false});
      window.mount=id => {
        window.frontend=window.driverModule.mount(document.body, {
          driverInstanceId:id,
          instanceConfigRef:'saved-'+id,
          i18n:{get locale(){return window.locale;},subscribe(fn){window.translate=fn;return ()=>{};}},
          async invoke(operation,args){
            window.calls.push({operation,args});
            if(operation==='serial.configure'){window.interpretation={...(window.interpretation??{}),...args};return receipt({config:{port:'COM1',baudRate:115200,dataBits:8,stopBits:1,parity:'none',dtr:false,rts:false,...window.interpretation}});}
            if(operation==='serial.status')return receipt({config:{port:'COM1',baudRate:115200,dataBits:8,stopBits:1,parity:'none',dtr:false,rts:false,...(window.interpretation??{})},
              connected:true,connectionId:window.connection,signals:{dtr:false,rts:false},rxBytes:window.rx,txBytes:0,commandQueue:0});
            if(operation==='serial.read')return receipt({text:window.replay??'',bytes:window.replay?.length??0,cursor:window.cursor,
              gap:window.gap??false,closed:false,more:false,connectionId:window.connection});
            if(operation==='serial.read_hex')return receipt({hex:Array.from(new TextEncoder().encode(window.rawOutput),b=>b.toString(16).padStart(2,'0')).join(''),
              bytes:window.rawOutput.length,cursor:window.cursor,gap:false,closed:false,more:false,connectionId:window.connection});
            throw new Error('unexpected operation '+operation);
          },
          async subscribeEvents(fn){window.event=fn;return ()=>{};}
        });
        window.root=document.body.firstElementChild.shadowRoot;
      };
      window.emit=async text => {window.rawOutput+=text;window.rx+=new TextEncoder().encode(text).length;
        await window.event({name:'serial.output',payload:{text,bytes:new TextEncoder().encode(text).length,cursor:++window.cursor,connectionId:window.connection}});};
      window.mount('one');
    })()`);
    await waitForValue(client, `root.querySelector('.connect-button').dataset.connected === 'true'`);
    assert.deepEqual(await checks(), [false, false]);
    assert.deepEqual(await run(`Array.from(root.querySelectorAll('.receive-newlines label'), label=>label.textContent)`), ["LF 补 CR", "CR 补 LF"]);
    assert.equal(await run(`root.querySelector('.receive-newlines select') === null`), true);
    await emit("A\nB\nC");
    assert.deepEqual((await rows()).slice(0, 3), ["A", " B", "  C"], "default is the original terminal cursor behavior");

    await change("lf"); await clear();
    await emit("A\n"); await emit("B\nC");
    assert.deepEqual((await rows()).slice(0, 3), ["A", "B", "C"], "LF-only output starts each line at column zero");
    await clear(); await emit("A\r"); await emit("\nB\r"); await emit("\nC");
    assert.deepEqual((await rows()).slice(0, 3), ["A", "B", "C"], "split CRLF is not doubled");

    await change("cr"); await clear();
    await emit("A\rB\rC");
    assert.deepEqual((await rows()).slice(0, 3), ["A", "B", "C"], "CR-only output gains line feeds");
    await change("both"); await clear();
    await emit("A\nB\r"); await emit("\nC\rD");
    assert.deepEqual((await rows()).slice(0, 4), ["A", "B", "C", "D"]);

    // Saving and HEX keep the exact input, not the normalized display.
    await run(`(() => {
      root.querySelectorAll('.modes button')[1].click();
      window.saved=[]; URL.createObjectURL=blob=>{window.saved.push(blob);return 'blob:receive';};
      URL.revokeObjectURL=()=>{}; HTMLAnchorElement.prototype.click=function(){};
      root.querySelector('.export-receive').click();
    })()`);
    assert.equal(await run(`window.saved[0].text()`), "A\nB\r\nC\rD");
    assert.equal(await run(`root.querySelector('.uart-output').textContent`), "A\nB\nC\nD");
    assert.equal(await run(`root.querySelector('.stats').textContent`), await run(`'接收 '+window.rx+' B · 发送 0 B'`));
    await run(`root.querySelector('[data-control=format] button[data-value=hex]').click()`);
    await waitForValue(client, `root.querySelector('.uart-output').textContent.includes('41 0a 42 0d 0a 43 0d 44')`);
    assert.equal(await run(`window.calls.some(c=>!['serial.status','serial.read','serial.read_hex','serial.configure'].includes(c.operation))`), false);

    // Settings remain available while connected; selecting them never changes UART parameters.
    await run(`root.querySelector('.settings-toggle').click()`);
    await waitForValue(client, `getComputedStyle(root.querySelector('.settings-panel')).opacity === '1'`);
    assert.deepEqual(await run(`Array.from(root.querySelectorAll('.receive-newlines input'), input=>input.disabled)`), [false, false]);
    await screenshot("receive-newlines-dark");
    await run(`document.body.style.width='360px'`);
    assert.equal(await run(`root.querySelector('.layout').scrollWidth <= 360`), true);
    assert.equal(await run(`root.querySelector('.receive-newlines').getBoundingClientRect().right <= 360`), true);
    await screenshot("receive-newlines-narrow");
    await run(`window.locale='en';window.translate()`);
    assert.deepEqual(await run(`Array.from(root.querySelectorAll('.receive-newlines label'), label=>label.textContent)`), ["Complete LF with CR", "Complete CR with LF"]);

    await run(`document.body.style.width='1000px'`);
    // Exercise each independent checkbox, including both checked and neither checked.
    await run(`root.querySelector('[name=receiveCompleteLF]').click()`);
    assert.deepEqual(await checks(), [false, true]);
    await run(`root.querySelector('[name=receiveCompleteCR]').click()`);
    assert.deepEqual(await checks(), [false, false]);
    await run(`root.querySelector('[name=receiveCompleteLF]').click();root.querySelector('[name=receiveCompleteCR]').click()`);
    assert.deepEqual(await checks(), [true, true]);

    await run(`window.frontend.unmount();window.mount('one')`);
    await waitForValue(client, `root.querySelector('.connect-button').dataset.connected === 'true'`);
    assert.deepEqual(await checks(), [true, true]);
    await run(`window.frontend.unmount();window.mount('two')`);
    await waitForValue(client, `root.querySelector('.connect-button').dataset.connected === 'true'`);
    assert.deepEqual(await checks(), [false, false]);

    // Replay, clear and new connections do not inherit another stream's CR state.
    // The fixture retains the UART mode configured above; xterm paints only in its visible view.
    await run(`root.querySelector('.modes button').click()`);
    await waitForValue(client, `root.querySelector('.modes button').getAttribute('aria-pressed') === 'true'`);
    await change("both"); await emit("\r"); await clear(); await emit("\nZ");
    await waitForValue(client, `root.querySelector('.xterm-rows > div:nth-child(2)')?.textContent.trim() === 'Z'`);
    assert.deepEqual((await rows()).slice(0, 2), ["", "Z"]);
    await emit("\r");
    await run(`(async()=>{window.replay='\\nR';window.gap=true;await window.event({name:'runtime.gap',payload:{}});})()`);
    await waitForValue(client, `root.querySelector('.xterm-screen').textContent.includes('R')`);
    assert.equal((await rows()).indexOf("R"), 3, "the replay LF survives reset after the gap notice");
    await emit("\r");
    await run(`(async()=>{window.gap=false;window.connection='new';window.replay='\\nN';window.cursor=1;
      await window.event({name:'serial.status',payload:{}});})()`);
    await waitForValue(client, `root.querySelector('.xterm-rows > div:nth-child(2)')?.textContent.trim() === 'N'`);
    assert.deepEqual((await rows()).slice(0, 2), ["", "N"]);
    await run(`window.frontend.unmount()`);
  } finally {
    await session.close();
    await new Promise(resolve => server.close(resolve));
  }
});
