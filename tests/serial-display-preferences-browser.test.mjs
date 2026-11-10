/** Real Serial frontend, recreated runtime IDs and a synthetic embedded device; no hardware I/O. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import { evaluate, findBrowser, openSession, waitForValue } from "./support/browser-harness.mjs";

async function serialSession(browserPath, { delayStatus = false } = {}) {
  const artifact = new URL("../drivers/driver-serial/dist/frontend/index.js", import.meta.url);
  const server = http.createServer((request, response) => {
    if (request.url === "/serial.js") {
      response.setHeader("Content-Type", "text/javascript");
      response.end(fs.readFileSync(artifact)); return;
    }
    response.setHeader("Content-Type", "text/html");
    response.end(String.raw`<!doctype html><html><body style="margin:0;width:1000px;height:760px"><script type="module">
      import {mount} from '/serial.js';
      window.calls=[]; window.cursor=0; window.input=''; window.submitted=[];
      window.id=crypto.randomUUID(); window.port=new URL(location.href).searchParams.get('port')??'COM6';
      window.configRef=new URL(location.href).searchParams.get('config')??'saved-serial-one';
      window.receipt=output=>({status:'completed',output,error:null});
      window.api={driverInstanceId:window.id,instanceConfigRef:window.configRef,
        async invoke(operation,args){
          calls.push({operation,args});
          if(operation==='serial.configure'){window.interpretation={...(window.interpretation??{}),...args};return receipt({config:{port,baudRate:115200,dataBits:8,stopBits:1,parity:'none',dtr:false,rts:false,...window.interpretation}});}
          if(operation==='serial.status'){
            if(new URL(location.href).searchParams.has('delay'))await new Promise(resolve=>{window.releaseStatus=resolve;});
            return receipt({config:{port,baudRate:115200,dataBits:8,stopBits:1,parity:'none',dtr:false,rts:false,...(window.interpretation??{})},
              connected:true,connectionId:'connection',signals:{dtr:false,rts:false},rxBytes:0,txBytes:0,commandQueue:0});
          }
          if(operation==='serial.read')return receipt({text:'',bytes:0,cursor:0,gap:false,closed:false,more:false,connectionId:'connection'});
          if(operation==='serial.read_hex')return receipt({hex:'',bytes:0,cursor:0,gap:false,closed:false,more:false,connectionId:'connection'});
          if(operation==='serial.write'){
            let echo='';
            for(const char of args.text){
              if(char==='\b'){if(window.input){window.input=window.input.slice(0,-1);echo+='\b \b';}}
              else if(char==='\x7f'){/* This embedded firmware accepts BS, not DEL. */}
              else if(char==='\r'){submitted.push(window.input);window.input='';echo+='\r\n';}
              else{window.input+=char;echo+=char;}
            }
            if(echo)await window.event({name:'serial.output',payload:{text:echo,bytes:echo.length,cursor:++cursor,connectionId:'connection'}});
            return receipt({bytes:args.text.length});
          }
          throw new Error('Unexpected operation '+operation);
        },
        async subscribeEvents(listener){window.event=listener;return()=>{};}
      };
      window.frontend=mount(document.body,api);
      window.root=document.body.firstElementChild.shadowRoot;
    </script></body></html>`);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const session = await openSession({ browserPath });
  const { client } = session;
  const run = source => evaluate(client, source);
  const wait = source => waitForValue(client, source);
  await client.send("Page.enable"); await client.send("Runtime.enable");
  await client.send("Emulation.setFocusEmulationEnabled", { enabled: true });
  const url = `http://127.0.0.1:${server.address().port}/`;
  await client.send("Page.navigate", { url: url + (delayStatus ? '?delay=1' : '') });
  await wait(delayStatus ? `!!window.releaseStatus` : `window.root?.querySelector('.connect-button').dataset.connected==='true'`);
  return { client, run, wait, url, close: async () => { await session.close(); await new Promise(resolve => server.close(resolve)); } };
}

test("Serial display preferences survive an application reload with a new runtime instance ID", async t => {
  const browserPath = findBrowser(); if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  const { client, run, wait, url, close } = await serialSession(browserPath);
  try {
    const previousId = await run(`window.id`);
    await run(`root.querySelector('.settings-toggle').click();root.querySelector('[name=receiveCompleteLF]').click();root.querySelector('[name=receiveCompleteCR]').click()`);
    await run(`root.querySelectorAll('.modes button')[1].click();
      root.querySelector('[data-control=format] button[data-value=hex]').click();
      root.querySelector('[data-control=timestamps] input').click();
      root.querySelector('[data-control=autoscroll] input').click();
      root.querySelector('[name=backspaceKey]').value='del';root.querySelector('[name=backspaceKey]').dispatchEvent(new Event('change'));
      root.querySelector('.line-ending').value='cr';root.querySelector('.line-ending').dispatchEvent(new Event('change'));
      root.querySelector('.send-mode button[data-value=hex]').click();
      root.querySelector('.uart-send input[type=number]').value='250';
      root.querySelector('.uart-send textarea').value='do not restore or send this';`);
    assert.deepEqual(await run(`Array.from(root.querySelectorAll('.receive-newlines input'),input=>input.checked)`), [true, true]);
    await client.send("Page.reload");
    await wait(`window.root?.querySelector('.connect-button').dataset.connected==='true'`);
    assert.notEqual(await run(`window.id`), previousId, "restart recreates runtime instance IDs");
    assert.deepEqual(await run(`Array.from(root.querySelectorAll('.receive-newlines input'),input=>input.checked)`), [true, true],
      "the same saved instance must retain both newline options after restart");
    assert.deepEqual(await run(`({view:!root.querySelector('.uart').hidden,
      hex:root.querySelector('[data-control=format] button[data-value=hex]').getAttribute('aria-pressed'),
      timestamps:root.querySelector('[data-control=timestamps] input').checked,
      autoScroll:root.querySelector('[data-control=autoscroll] input').checked,
      backspace:root.querySelector('[name=backspaceKey]').value})`),
      { view: true, hex: 'true', timestamps: true, autoScroll: false, backspace: 'del' });
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), '');
    assert.equal(await run(`root.querySelector('.line-ending').value`), 'cr');
    assert.equal(await run(`root.querySelector('.send-mode button[data-value=text]').getAttribute('aria-pressed')`), 'true');
    assert.equal(await run(`root.querySelector('.uart-send input[type=number]').value`), '100');
    assert.equal(await run(`calls.some(c=>c.operation==='serial.write'||c.operation==='serial.write_hex')`), false,
      "restoring UI choices must never send anything");
    await run(`root.querySelector('.uart-send textarea').value='probe';
      root.querySelector('.uart-send textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));`);
    await wait(`calls.some(c=>c.operation==='serial.write')`);
    assert.deepEqual(await run(`calls.find(c=>c.operation==='serial.write').args`), { text: 'probe', lineEnding: 'cr' });
    await client.send('Page.navigate', { url: url + '?config=saved-serial-two&port=COM6' });
    await wait(`window.root?.querySelector('.connect-button').dataset.connected==='true'`);
    assert.deepEqual(await run(`Array.from(root.querySelectorAll('.receive-newlines input'),input=>input.checked)`), [false, false]);
    assert.equal(await run(`root.querySelector('[name=backspaceKey]').value`), 'bs');
    assert.equal(await run(`root.querySelector('.line-ending').value`), 'crlf');
    await client.send('Page.navigate', { url: url + '?config=saved-serial-one&port=COM9' });
    await wait(`window.root?.querySelector('.connect-button').dataset.connected==='true'`);
    assert.deepEqual(await run(`Array.from(root.querySelectorAll('.receive-newlines input'),input=>input.checked)`), [true, true],
      "preferences belong to the saved instance, independent of its COM assignment");
    assert.equal(await run(`root.querySelector('.line-ending').value`), 'cr');
  } finally { await close(); }
});

test("Serial Backspace sends BS and the embedded device echo erases the character", async t => {
  const browserPath = findBrowser(); if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  const { client, run, wait, close } = await serialSession(browserPath);
  try {
    await run(`root.querySelector('.xterm-helper-textarea').focus()`);
    await client.send("Input.insertText", { text: "abc" });
    await wait(`window.input==='abc'&&root.querySelector('.xterm-screen').textContent.includes('abc')`);
    const writesBefore = await run(`window.calls.filter(call=>call.operation==='serial.write').length`);
    await client.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8 });
    await client.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8 });
    const sent = await run(`window.calls.filter(call=>call.operation==='serial.write').at(-1).args`);
    assert.equal(await run(`window.calls.filter(call=>call.operation==='serial.write').length`), writesBefore + 1,
      "one Backspace must send exactly one byte, without xterm's additional DEL");
    assert.deepEqual(sent, { text: "\b", lineEnding: "none" }, "embedded firmware needs BS (0x08), without an added line ending");
    await wait(`window.input==='ab'&&!root.querySelector('.xterm-screen').textContent.includes('abc')`);
    await client.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r" });
    await client.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await wait(`window.submitted.length===1`);
    assert.deepEqual(await run(`window.submitted`), ["ab"]);

    await run(`root.querySelector('.settings-toggle').click();
      root.querySelector('[name=backspaceKey]').value='del';root.querySelector('[name=backspaceKey]').dispatchEvent(new Event('change'));
      root.querySelector('.settings-toggle').click();root.querySelector('.xterm-helper-textarea').focus()`);
    const beforeDEL = await run(`window.calls.filter(call=>call.operation==='serial.write').length`);
    await client.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8 });
    await client.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8 });
    await wait(`window.calls.filter(call=>call.operation==='serial.write').length===${beforeDEL + 1}`);
    assert.deepEqual(await run(`window.calls.filter(call=>call.operation==='serial.write').at(-1).args`), {text:'\x7f',lineEnding:'none'});

    // A literal DEL pasted into a BS-configured terminal is still DEL.
    await run(`root.querySelector('[name=backspaceKey]').value='bs';root.querySelector('[name=backspaceKey]').dispatchEvent(new Event('change'));
      const data=new DataTransfer();data.setData('text/plain','x\x7fy');
      root.querySelector('.xterm-helper-textarea').dispatchEvent(new ClipboardEvent('paste',{clipboardData:data,bubbles:true,cancelable:true,composed:true}));`);
    await wait(`window.calls.filter(call=>call.operation==='serial.write').at(-1).args.text==='x\x7fy'`);

    // Backspace in the UART sender is local native editing, with no device write.
    await run(`root.querySelectorAll('.modes button')[1].click();const input=root.querySelector('.uart-send textarea');
      input.value='abc';input.focus();input.setSelectionRange(3,3)`);
    const beforeUART = await run(`window.calls.filter(call=>call.operation==='serial.write').length`);
    await client.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8 });
    await client.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8 });
    assert.equal(await run(`root.querySelector('.uart-send textarea').value`), 'ab');
    assert.equal(await run(`window.calls.filter(call=>call.operation==='serial.write').length`), beforeUART);
  } finally { await close(); }
});

test("display edits made before the first status response are saved without being overwritten", async t => {
  const browserPath = findBrowser(); if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  const { run, wait, close } = await serialSession(browserPath, { delayStatus: true });
  try {
    await run(`root.querySelector('.settings-toggle').click();root.querySelector('[name=receiveCompleteLF]').click();window.releaseStatus()`);
    await wait(`window.root?.querySelector('.connect-button').dataset.connected==='true'`);
    assert.equal(await run(`root.querySelector('[name=receiveCompleteLF]').checked`), true);
    assert.equal(await run(`JSON.parse(localStorage.getItem('coterm.serial.display.v1:config:saved-serial-one')).receiveNewlines`), 'lf');
    assert.equal(await run(`calls.some(c=>c.operation==='serial.write')`), false);
  } finally { await close(); }
});
