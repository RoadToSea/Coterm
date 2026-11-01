/** Real SSH bundle + shared SDK UI; the fixture implements only the plugin data seam. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("SSH blocks render backend snapshots, retain selection, submit commands and recover after gaps", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No browser; SSH command blocks were not browser-verified"); return; }
  const artifact = new URL("../plugins/plugin-ssh/dist/frontend/index.js", import.meta.url);
  const built = fs.statSync(artifact).mtimeMs;
  for (const file of ["index.ts", "blocks.ts", "i18n.ts"]) {
    assert.ok(built >= fs.statSync(new URL(`../plugins/plugin-ssh/frontend/src/${file}`, import.meta.url)).mtimeMs,
      "build the SSH frontend before running its browser tests");
  }
  assert.ok(built >= fs.statSync(new URL("../packages/plugin-sdk/src/components/block-list.ts", import.meta.url)).mtimeMs);
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({ browserPath, totalTimeoutMs: 45_000 });
  const { client } = session;
  const run = source => evaluate(client, source);
  const screenshot = async name => {
    const directory = process.env.COTERM_SSH_BLOCKS_SCREENSHOT_DIR;
    if (!directory) return;
    fs.mkdirSync(directory, { recursive: true });
    const result = await client.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(path.join(directory, `${name}.png`), Buffer.from(result.data, "base64"));
  };
  const errors = [];
  try {
    await client.send("Runtime.enable");
    client.on("Runtime.exceptionThrown", event => errors.push(event.exceptionDetails.text));
    const tokens = fs.readFileSync(new URL("../apps/desktop/src/index.css", import.meta.url), "utf8")
      .match(/(?::root|\.dark)\s*\{[^}]*\}/g)?.join("\n");
    assert.ok(tokens, "use the real desktop theme tokens");
    await run(`document.documentElement.classList.add('dark');const theme=document.createElement('style');theme.textContent=${JSON.stringify(tokens)};document.head.append(theme)`);
    await client.send("Emulation.setDeviceMetricsOverride", { width: 1000, height: 720, deviceScaleFactor: 1, mobile: false });
    await run(`(async()=>{
      document.body.style.cssText='margin:0;height:100vh;width:100vw';
      window.locale='en'; window.calls=[]; window.cancelled=[]; window.copied=''; window.version=1; window.epoch='connection-one';
      window.block=(id,index,extra={})=>({id,index,command:'echo '+id,actor:'human',channel:'pty',state:'completed',exitCode:0,
        output:'reply '+id,stderr:'',commandTruncated:false,outputTruncated:false,...extra});
      window.backend=[block('one',1),block('two',2,{actor:'robot',channel:'exec',state:'running',exitCode:null})];
      window.fail=false; window.delay=false;
      Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{window.copied=text;}}});
      const receipt=output=>({operationId:'op-fixture',status:'completed',output,error:null,proposalId:null,waitExpired:false});
      const api={pluginInstanceId:'ssh-one',i18n:{get locale(){return window.locale},subscribe(fn){window.translate=fn;return()=>{window.localeStopped=true}}},
        async invoke(operation,args,options){
          calls.push({operation,args,options});
          if(operation==='ssh.status')return receipt({config:{host:'localhost',port:22,username:'operator'},connected:true,connectionId:'transport-one'});
          if(operation==='ssh.resize')return receipt({});
          if(operation==='ssh.read')return receipt({data:'',cursor:'0',gap:false,more:false,closed:false,connectionId:'transport-one'});
          if(operation==='ssh.blocks'){
            if(window.delay)await new Promise(resolve=>{window.release=resolve});
            if(window.fail)throw new Error('Temporary block read failure');
            if(window.changeOnPage && args.before!==undefined){window.version++;window.changeOnPage=false;}
            const result={epoch:window.epoch,revision:String(window.version),dropped:window.dropped??0,blocks:[],more:false,nextBefore:null};
            const matches=backend.filter(b=>args.before===undefined||b.index<args.before).sort((a,b)=>b.index-a.index);
            result.blocks=matches.slice(0,1);result.more=matches.length>1;result.nextBefore=result.more?result.blocks[0].index:null;
            return receipt(result);
          }
          if(operation==='ssh.command'){
            backend.push(block('submitted',3,{command:args.text,state:'running',exitCode:null}));window.version++;
            return {...receipt(null),operationId:'submitted',status:'running'};
          }
          throw new Error('Unexpected operation '+operation);
        },
        async subscribeEvents(fn){window.event=fn;return()=>{window.stopped=true}},
        async cancelOperation(id){cancelled.push(id);backend=backend.map(b=>b.id===id?{...b,state:'cancelled'}:b);window.version++;return receipt({})},
      };
      window.frontend=(await import(${JSON.stringify(moduleURL)})).mount(document.body,api);
      window.root=document.body.firstElementChild.shadowRoot;
      window.choices=[...root.querySelectorAll('.ssh-view-switch button')];
      window.rows=()=>[...root.querySelectorAll('.coterm-block')];
      window.gutters=()=>rows().map(row=>row.querySelector('.coterm-block-gutter'));
      window.tools=()=>[...root.querySelectorAll('.ssh-blocks-tools button')];
    })()`);
    await waitForValue(client, `root.querySelector('.bar span').textContent==='Connected'`);
    assert.equal(await run(`calls.filter(c=>c.operation==='ssh.blocks').length`), 0, "terminal mode loads no block history");
    await run(`choices[1].click()`);
    await waitForValue(client, `rows().length===2`);
    assert.deepEqual(await run(`rows().map(row=>row.dataset.blockId)`), ["one", "two"], "backend pages display oldest to newest");
    assert.equal(await run(`root.querySelector('.terminal-pane').hidden`), true);
    assert.equal(await run(`rows()[1].querySelector('.coterm-block-state').textContent`), "EXEC · running");
    await run(`gutters()[1].click(); window.keptRow=rows()[1]; window.keptButton=gutters()[1]; keptButton.focus()`);
    assert.equal(await run(`rows()[1].dataset.selected`), "true");
    assert.equal(await run(`tools()[1].disabled`), false);
    await screenshot("ssh-blocks-dark");
    const selectedColor = await run(`getComputedStyle(rows()[1]).backgroundColor`);
    await run(`document.documentElement.classList.remove('dark')`);
    assert.notEqual(await run(`getComputedStyle(rows()[1]).backgroundColor`), selectedColor);
    assert.equal(await run(`rows()[1].dataset.selected`), "true");
    await screenshot("ssh-blocks-light");
    await run(`document.documentElement.classList.add('dark')`);
    await run(`window.changeOnPage=true;window.previousReads=calls.filter(c=>c.operation==='ssh.blocks').length;
      for(let n=0;n<40;n++)window.event({name:'ssh.blocks.changed',payload:{}})`);
    await waitForValue(client, `calls.filter(c=>c.operation==='ssh.blocks').length>=previousReads+4`);
    assert.equal(await run(`rows()[1]===keptRow && rows()[1].dataset.selected==='true'`), true, "a changing page revision retries without losing selection");
    assert.ok(await run(`calls.filter(c=>c.operation==='ssh.blocks').length<=previousReads+6`), "a burst of events is coalesced");
    await run(`backend=backend.map(b=>b.id==='two'?{...b,output:'updated <img src=x onerror=alert(1)>',stderr:'diagnostic'}:b); window.version++; window.event({name:'ssh.blocks.changed',payload:{}})`);
    await waitForValue(client, `rows()[1].querySelector('.coterm-block-output').textContent.includes('diagnostic')`);
    assert.equal(await run(`rows()[1]===keptRow && gutters()[1]===keptButton && root.activeElement===keptButton && rows()[1].dataset.selected==='true'`), true);
    assert.equal(await run(`root.querySelector('img')===null`), true);
    await run(`tools()[0].click()`);
    await waitForValue(client, `window.copied.includes('diagnostic')`);
    assert.match(await run(`window.copied`), /echo two\nupdated <img/);
    await run(`tools()[1].click()`);
    await waitForValue(client, `rows()[1].querySelector('.coterm-block-state').textContent.includes('cancelled')`);
    assert.deepEqual(await run(`window.cancelled`), ["two"]);
    assert.equal(await run(`tools()[1].disabled`), true);

    await run(`window.locale='zh-CN';window.translate()`);
    assert.equal(await run(`choices[1].textContent`), "消息块");
    assert.match(await run(`rows()[1].querySelector('.coterm-block-state').textContent`), /已取消/);
    await run(`root.querySelector('.ssh-blocks-compose input').value='echo from-ui'; root.querySelector('.ssh-blocks-compose').requestSubmit()`);
    await waitForValue(client, `rows().length===3`);
    assert.deepEqual(await run(`calls.filter(c=>c.operation==='ssh.command').map(c=>({args:c.args,options:c.options}))`), [{ args: { text: "echo from-ui" }, options: { waitTimeoutMs: 0 } }]);
    assert.equal(await run(`root.querySelector('.ssh-blocks-compose button').disabled`), true, "shared PTY has one active command");
    assert.equal(await run(`root.querySelector('.ssh-blocks-compose input').value`), "");
    await run(`backend=backend.filter(b=>b.id!=='two').map(b=>b.id==='submitted'?{...b,state:'completed',exitCode:0,output:'done',outputTruncated:true}:b);window.dropped=1;window.version++;window.event({name:'runtime.gap',payload:{}})`);
    await waitForValue(client, `rows().length===2 && rows()[1].querySelector('.coterm-block-output').textContent.includes('已截断')`);
    assert.equal(await run(`tools()[0].disabled`), true, "evicted blocks leave selection");
    assert.match(await run(`root.querySelector('.ssh-blocks-tools').textContent`), /较早的 1 个块/);
    assert.equal(await run(`root.querySelector('.ssh-blocks-compose button').disabled`), false);
    for (const width of [768, 320]) {
      await client.send("Emulation.setDeviceMetricsOverride", { width, height: 720, deviceScaleFactor: 1, mobile: false });
      assert.equal(await run(`document.documentElement.scrollWidth<=innerWidth && root.querySelector('.ssh-blocks-view').scrollWidth<=root.querySelector('.ssh-blocks-view').clientWidth`), true, `no overflow at ${width}px`);
    }
    await screenshot("ssh-blocks-narrow");
    await run(`window.fail=true;window.event({name:'ssh.blocks.changed',payload:{}})`);
    await waitForValue(client, `!root.querySelector('.ssh-blocks-error').hidden`);
    assert.match(await run(`root.querySelector('.ssh-blocks-error').textContent`), /Temporary block read failure/);
    assert.equal(await run(`rows().length`), 2, "read failure preserves the last valid history");
    await run(`window.fail=false;window.epoch='connection-two';backend=[];window.version++;root.querySelector('.ssh-blocks-error button').click()`);
    await waitForValue(client, `rows().length===0 && root.querySelector('.ssh-blocks-error').hidden`);
    assert.match(await run(`root.querySelector('.coterm-block-empty').textContent`), /暂无消息块/);

    await run(`backend=[block('lost',1,{state:'running',exitCode:null})];window.version++;window.event({name:'ssh.blocks.changed',payload:{}})`);
    await waitForValue(client, `rows().length===1`);
    await run(`gutters()[0].click();window.event({name:'runtime.closed',payload:{}})`);
    await waitForValue(client, `rows()[0].querySelector('.coterm-block-state').textContent.includes('结果未知')`);
    assert.equal(await run(`tools()[1].disabled && root.querySelector('.ssh-blocks-compose button').disabled`), true, "closed transport disables writes and leaves execution unknown");

    await run(`window.delay=true;window.event({name:'ssh.blocks.changed',payload:{}})`);
    await waitForValue(client, `typeof window.release==='function'`);
    await run(`window.oldButton=choices[1];window.frontend.unmount();window.release();window.event({name:'ssh.blocks.changed',payload:{}});oldButton.click()`);
    assert.equal(await run(`document.body.childElementCount`), 0);
    assert.equal(await run(`window.stopped && window.localeStopped`), true);
    assert.deepEqual(errors, []);
  } finally { await session.close(); }
});
