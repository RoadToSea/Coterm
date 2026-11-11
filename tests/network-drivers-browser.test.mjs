import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { assertFreshDriverBuild, evaluate, findBrowser, openSession, rootDir, waitForValue } from "./support/browser-harness.mjs";

for (const protocol of ["tcp","telnet"]) {
  test(`${protocol} uses the shared workbench with explicit connect, epochs and disposable status recovery`,async t => {
    const browserPath=findBrowser(); if (!browserPath) {t.skip("No Chrome/Edge binary");return;}
    assertFreshDriverBuild(protocol);
    const artifact=new URL(`../crates/coterm-driver/dist/${protocol}/frontend/index.js`,import.meta.url);
    const moduleURL=`data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
    const session=await openSession({browserPath,totalTimeoutMs:45000});const {client}=session;
    const run=source=>evaluate(client,source),wait=source=>waitForValue(client,source);
    const key=async (key,code,windowsVirtualKeyCode) => {
      for (const type of ["keyDown","keyUp"]) await client.send("Input.dispatchKeyEvent",{type,key,code,windowsVirtualKeyCode});
    };
    const screenshot=async name => {
      if (!process.env.COTERM_UI_SCREENSHOTS) return;
      const directory=path.join(rootDir,".agent-debug/network-drivers");fs.mkdirSync(directory,{recursive:true});
      const {data}=await client.send("Page.captureScreenshot",{format:"png"});
      fs.writeFileSync(path.join(directory,`${protocol}-${name}.png`),Buffer.from(data,"base64"));
    };
    try {
      await client.send("Page.enable");await client.send("Runtime.enable");
      await client.send("Emulation.setDeviceMetricsOverride",{width:960,height:720,deviceScaleFactor:1,mobile:false});
      await client.send("Emulation.setEmulatedMedia",{features:[{name:"prefers-reduced-motion",value:"reduce"}]});
      await client.send("Emulation.setFocusEmulationEnabled",{enabled:true});
      await run(`(async()=>{
        document.body.style.cssText='margin:0;width:960px;height:720px;background:#1e1e1e';
        window.protocol=${JSON.stringify(protocol)};window.module=await import(${JSON.stringify(moduleURL)});
        window.calls=[];window.submitted=[];window.saved=[];window.listeners=new Set();
        window.i18n={locale:'zh-CN',subscribe(fn){listeners.add(fn);return()=>listeners.delete(fn)}};
        window.view=module.configure(document.body,{i18n,submit:async config=>submitted.push(config)});
        window.root=document.body.firstElementChild.shadowRoot;
      })()`);
      assert.equal(await run(`root.querySelector('[name=port]').value`),protocol==="telnet"?"23":"8080");
      assert.equal(await run(`!!root.querySelector('[name=username]')`),protocol==="telnet");
      assert.equal(await run(`root.querySelector('[name=autoReconnect]').checked`),false);
      assert.equal(await run(`root.querySelector('[name=localEcho]').value`),protocol==="telnet"?"auto":"on");
      assert.equal(await run(`root.querySelector('[name=baudRate]')===null&&root.querySelector('[name=dtr]')===null`),true);
      await run(`root.querySelector('[name=host]').value='127.0.0.1';root.querySelector('[name=proxyType]').value='http';root.querySelector('[name=proxyType]').dispatchEvent(new Event('change',{bubbles:true}));root.querySelector('[name=proxyHost]').value='localhost';root.querySelector('[name=proxyPort]').value='8081'`);
      await screenshot("configuration");
      await run(`root.querySelector('form').dispatchEvent(new Event('submit',{cancelable:true}))`);
      await wait(`submitted.length===1`);
      assert.deepEqual(await run(`submitted[0].proxy`),{type:"http",host:"localhost",port:8081});
      assert.equal(await run(`'password' in submitted[0]`),false);
      await run(`view.unmount();window.config={...submitted[0],proxy:{type:'direct'},prompt:'root# '};
        window.connected=false;window.reconnecting=false;window.epoch=null;window.remoteEcho=true;window.unsubscribed=0;
        window.receipt=output=>({status:'completed',error:null,output});
        window.snapshot=()=>({config:{...config},connected,reconnecting,connectionId:epoch,rxBytes:20,txBytes:0,commandQueue:0,remoteEcho});
        window.api={i18n,driverInstanceId:protocol+'-instance',instanceConfigRef:protocol+'-saved',
          async saveConfiguration(value){if(window.failSave)throw new Error('save denied');saved.push({...value})},
          async subscribeEvents(fn){window.event=fn;return()=>{unsubscribed++}},
          async invoke(operation,args){calls.push({operation,args});const name=operation.slice(protocol.length+1);
            if(name==='status')return receipt(snapshot());
            if(name==='configure'){Object.assign(config,args);return receipt(snapshot())}
            if(name==='connect'){connected=true;reconnecting=false;epoch='epoch-1';return receipt({connected:true,connectionId:epoch})}
            if(name==='disconnect'){connected=false;reconnecting=false;return receipt(snapshot())}
            if(name==='read')return receipt({connectionId:epoch,text:args.cursor?'':'hello\\r\\nworld\\r\\nroot# ',bytes:args.cursor?0:20,cursor:1,gap:false,closed:!connected,more:false});
            if(name==='read_hex')return receipt({connectionId:epoch,hex:args.cursor?'':'00ff80',bytes:args.cursor?0:3,cursor:1,gap:false,closed:!connected,more:false});
            if(name==='write'||name==='write_hex')return receipt({bytes:args.text?.length??args.hex.length/2});
            if(name==='resize'||name==='set_selection')return receipt({ok:true});
            throw new Error('Unexpected operation '+operation);
          }};
        window.mount=()=>{window.view=module.mount(document.body,api);window.root=document.body.firstElementChild.shadowRoot};mount();`);
      await wait(`!root.querySelector('.connect-button').disabled`);
      assert.equal(await run(`calls.some(call=>call.operation===protocol+'.connect')`),false,"mounting never connects");
      if (protocol==="tcp") {
        await run(`root.querySelector('.settings-toggle').click();let host=root.querySelector('[name=host]');host.value='board.local';host.dispatchEvent(new Event('input',{bubbles:true}));window.failSave=true;root.querySelector('.connect-button').click()`);
        await wait(`root.querySelector('.coterm-toast')?.textContent.includes('save denied')&&!root.querySelector('.connect-button').disabled`);
        assert.equal(await run(`calls.some(call=>call.operation===protocol+'.connect')`),false,"save failure must prevent connecting");
        await run(`window.failSave=false;root.querySelector('.connect-button').click()`);
      } else await run(`root.querySelector('.connect-button').click()`);
      await wait(`root.querySelector('.connect-button').dataset.connected==='true'`);
      if (protocol==="tcp") {
        await wait(`root.querySelector('.uart-output').textContent.includes('world')`);
        await run(`if(root.querySelector('.settings-toggle').getAttribute('aria-expanded')==='true')root.querySelector('.settings-toggle').click();root.querySelector('.uart-bar .format-switch [data-value=hex]').click()`);
        await wait(`root.querySelector('.uart-output').textContent.includes('00 ff 80')`);
        await run(`root.querySelector('.send-mode [data-value=hex]').click();root.querySelector('.uart-send textarea').value='00 ff 80';root.querySelector('.uart-send textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}))`);
        await wait(`calls.some(call=>call.operation==='tcp.write_hex')`);
        assert.deepEqual(await run(`calls.find(call=>call.operation==='tcp.write_hex').args`),{hex:"00ff80",connectionId:"epoch-1"});
        await screenshot("hex");
        await run(`root.querySelectorAll('.modes button')[0].click()`);
        await wait(`root.querySelectorAll('.modes button')[0].getAttribute('aria-pressed')==='true'&&!root.querySelector('.connect-button').disabled`);
      } else {
        assert.equal(await run(`root.querySelector('.modes').hidden`),true);
        assert.equal(await run(`root.querySelector('.uart').hidden&&root.querySelector('.uart-send').hidden`),true);
        assert.equal(await run(`root.querySelector('.ssh-jobs')===null&&root.querySelector('[name=exec]')===null`),true);
      }
      await wait(`root.querySelector('.xterm-rows').textContent.includes('hello')`);
      await run(`root.querySelector('.xterm-helper-textarea').focus()`);
      await client.send("Input.insertText",{text:"X"});
      await wait(`calls.some(call=>call.operation===protocol+'.write'&&call.args.text==='X')`);
      assert.equal(await run(`calls.filter(call=>call.operation===protocol+'.write').at(-1).args.connectionId`),"epoch-1");
      if (protocol==="tcp") await wait(`root.querySelector('.xterm-rows').textContent.includes('X')`);
      else {
        assert.equal(await run(`root.querySelector('.xterm-rows').textContent.includes('X')`),false,"server echo negotiation suppresses automatic local echo");
        await key("Enter","Enter",13);
        await wait(`calls.some(call=>call.operation==='telnet.write'&&call.args.text==='\\r\\n')`);
        await wait(`calls.some(call=>call.operation==='telnet.resize')`);
      }
      await key("Backspace","Backspace",8);
      await wait(`calls.some(call=>call.operation===protocol+'.write'&&call.args.text===${JSON.stringify(protocol==="telnet"?"\x7f":"\b")})`);
      await run(`window.i18n.locale='en';for(const fn of listeners)fn()`);
      assert.equal(await run(`root.querySelector('.settings-toggle').textContent`),"Settings");
      // No event is sent: the low frequency status recovery must correct a stale green button.
      await run(`window.connected=false;window.reconnecting=true`);
      await wait(`root.querySelector('.connect-button').textContent==='Stop reconnecting'`);
      await run(`root.querySelector('.connect-button').click()`);
      await wait(`root.querySelector('.connect-button').textContent==='Connect'`);
      assert.equal(await run(`calls.filter(call=>call.operation===protocol+'.connect').length`),1);
      await client.send("Emulation.setDeviceMetricsOverride",{width:360,height:720,deviceScaleFactor:1,mobile:false});
      await run(`document.body.style.width='360px';if(root.querySelector('.settings-toggle').getAttribute('aria-expanded')!=='true')root.querySelector('.settings-toggle').click()`);
      await wait(`root.querySelector('.settings-panel').getAttribute('aria-hidden')==='false'`);
      await screenshot("narrow");
      assert.equal(await run(`root.querySelector('.settings-panel').scrollWidth<=360`),true);
      await run(`view.unmount();window.afterUnmount=calls.length`);
      assert.equal(await run(`unsubscribed`),1);
      assert.equal(await run(`listeners.size`),0);
      await run(`new Promise(resolve=>setTimeout(resolve,2200))`);
      assert.equal(await run(`calls.length`),await run(`afterUnmount`),"unmount removes the polling timer");
    } finally {await session.close();}
  });
}
