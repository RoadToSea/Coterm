/** Real xterm + SSH bundle: trusted boundaries, same-screen selection and Agent context. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("SSH integrates command blocks into the real terminal and publishes only human selection", async t => {
  const browserPath=findBrowser();
  if(!browserPath){t.skip("No browser; terminal blocks were not verified");return;}
  const artifact=new URL("../plugins/plugin-ssh/dist/frontend/index.js",import.meta.url);
  const built=fs.statSync(artifact).mtimeMs;
  for(const file of ["index.ts","blocks.ts","jobs.ts","i18n.ts"]){
    assert.ok(built>=fs.statSync(new URL("../plugins/plugin-ssh/frontend/src/"+file,import.meta.url)).mtimeMs,"build SSH first");
  }
  const moduleURL="data:text/javascript;base64,"+fs.readFileSync(artifact).toString("base64");
  const session=await openSession({browserPath,totalTimeoutMs:60000});
  const {client}=session;
  const run=source=>evaluate(client,source);
  const screenshot=async name=>{
    const dir=process.env.COTERM_SSH_BLOCKS_SCREENSHOT_DIR;
    if(!dir)return;fs.mkdirSync(dir,{recursive:true});
    const result=await client.send("Page.captureScreenshot",{format:"png"});fs.writeFileSync(path.join(dir,name+".png"),Buffer.from(result.data,"base64"));
  };
  const errors=[];
  try{
    await client.send("Runtime.enable");
    client.on("Runtime.exceptionThrown",event=>errors.push(event.exceptionDetails.text));
    await client.send("Emulation.setDeviceMetricsOverride",{width:1100,height:760,deviceScaleFactor:1,mobile:false});
    const tokens=fs.readFileSync(new URL("../apps/desktop/src/index.css",import.meta.url),"utf8").match(/(?::root|\.dark)\s*\{[^}]*\}/g)?.join("\n");
    await run("(async()=>{"+
      "document.body.style.cssText='margin:0;height:100vh;width:100vw';"+
      "const theme=document.createElement('style');theme.textContent="+JSON.stringify(tokens)+";document.head.append(theme);document.documentElement.classList.add('dark');"+
      "window.locale='en';window.calls=[];window.version=1;window.epoch='transport-one';window.context=[];window.fail=false;"+
      "window.block=(id,index,extra={})=>({id,index,command:'echo '+id,actor:'human',channel:'pty',state:'completed',exitCode:0,output:'reply '+id,stderr:'',commandTruncated:false,outputTruncated:false,...extra});"+
      "window.backend=[block('one',1),block('two',2,{actor:'robot'}),block('job-3',3,{channel:'exec',actor:'robot',state:'running',exitCode:null})];"+
      "window.frames=[];window.sequence=0;"+
      "window.add=(data='',annotation=null)=>{const frame={data:btoa(data),annotation,cursor:String(++sequence),connectionId:'transport-one'};frames.push(frame);return frame;};"+
      "window.bound=(id,kind)=>{const b=backend.find(b=>b.id===id);return {kind,id,index:b.index,actor:b.actor};};"+
      "add('',bound('one','begin'));add('reply one\\r\\n');add('',bound('one','end'));add('',bound('two','begin'));add('reply two\\r\\n');add('',bound('two','end'));"+
      "window.emit=(data='',annotation=null)=>event({name:'ssh.terminal.output',payload:add(data,annotation)});"+
      "const receipt=output=>({operationId:'fixture',status:'completed',output,error:null,proposalId:null,waitExpired:false});"+
      "const api={pluginInstanceId:'ssh-one',i18n:{get locale(){return window.locale},subscribe(fn){window.translate=fn;return()=>window.localeStopped=true}},"+
      "async invoke(operation,args,options){calls.push({operation,args,options});"+
      "if(operation==='ssh.status')return receipt({config:{host:'localhost',port:22,username:'operator'},connected:true,connectionId:'transport-one'});"+
      "if(operation==='ssh.resize'||operation==='ssh.input')return receipt({});"+
      "if(operation==='ssh.read'){const frame=frames.find(f=>BigInt(f.cursor)>BigInt(args.cursor));return receipt(frame?{...frame,more:frame!==frames.at(-1),gap:false,closed:false}:{data:'',cursor:String(sequence),more:false,gap:false,closed:false,connectionId:'transport-one'});}"+
      "if(operation==='ssh.blocks'){if(window.delay)await new Promise(resolve=>window.release=resolve);if(fail)throw new Error('Temporary block read failure');"+
      "if(window.changeOnPage&&args.before!==undefined){version++;window.changeOnPage=false;}"+
      "const matches=backend.filter(b=>args.before===undefined||b.index<args.before).sort((a,b)=>b.index-a.index);const page=matches.slice(0,1);"+
      "return receipt({epoch,revision:String(version),dropped:window.dropped??0,blocks:page,more:matches.length>1,nextBefore:matches.length>1?page[0].index:null});}"+
      "if(operation==='ssh.select_blocks'){if(window.missingSelect)throw new Error('operation_not_found: operation not found');window.context=args.ids;return receipt({selectedCount:context.length});}"+
      "if(operation==='ssh.output')return receipt({done:false,exitCode:null,cancelled:false,timedOut:false,stdout:{text:args.stdoutCursor==='0'?'streaming exec output\\n':'',cursor:'1',more:false},stderr:{text:'',cursor:'0',more:false}});"+
      "throw new Error('Unexpected operation '+operation);},"+
      "async subscribeEvents(fn){window.event=fn;return()=>window.stopped=true},async cancelOperation(){throw new Error('selection must never cancel');}};"+
      "window.frontend=(await import("+JSON.stringify(moduleURL)+")).mount(document.body,api);"+
      "window.root=document.body.firstElementChild.shadowRoot;window.gutters=()=>[...root.querySelectorAll('.coterm-terminal-number')];"+
      "})()");
    await waitForValue(client,"gutters().length===2 && root.querySelector('.bar span').textContent==='Connected'");
    assert.equal(await run("root.querySelector('.ssh-view-switch')===null"),true);
    assert.equal(await run("root.querySelector('.ssh-blocks-compose')===null && root.querySelector('#ssh-block-command')===null"),true,"the terminal is the only command input");
    assert.equal(await run("root.querySelector('.ssh-block-history').hidden"),true,"empty history does not reserve a footer row");
    assert.equal(await run("root.querySelector('.xterm-screen')!==null"),true);
    assert.equal(await run("frames.filter(frame=>frame.annotation).every(frame=>!('command' in frame.annotation)&&!('output' in frame.annotation))"),true,"boundary events do not carry command or output data");
    assert.deepEqual(await run("gutters().map(b=>b.textContent)"),["1","2"]);
    assert.equal(await run("root.querySelectorAll('.coterm-terminal-gutter > button:not(.coterm-terminal-number)').length"),1);
    assert.equal(await run("root.querySelector('.coterm-terminal-clear').disabled"),true);
    assert.deepEqual(await run("(()=>{const b=root.querySelector('.coterm-terminal-clear');const r=b.getBoundingClientRect();const s=getComputedStyle(b);return [b.textContent,b.getAttribute('aria-label'),Math.round(r.width),Math.round(r.height),s.borderRadius,s.backgroundColor,s.opacity]})()"),["×","Clear selection",16,16,"50%","rgb(255, 95, 87)","1"]);
    await run("window.gutterWidth=()=>root.querySelector('.coterm-terminal-gutter').getBoundingClientRect().width;window.singleDigitWidth=gutterWidth();backend=backend.map(b=>b.id==='two'?{...b,index:10}:b);version++;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"gutters()[1].textContent==='10' && gutterWidth()>=singleDigitWidth");
    await run("window.twoDigitWidth=gutterWidth();backend=backend.map(b=>b.id==='two'?{...b,index:4294967295}:b);version++;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"gutters()[1].textContent==='4294967295' && gutterWidth()>twoDigitWidth");
    assert.equal(await run("gutters().every(b=>b.scrollWidth<=b.clientWidth)"),true,"the largest supported block number fits without clipping");
    await run("backend=backend.map(b=>b.id==='two'?{...b,index:2}:b);version++;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"gutters()[1].textContent==='2' && gutterWidth()===singleDigitWidth");
    await waitForValue(client,"(()=>{const screen=root.querySelector('.xterm-screen').getBoundingClientRect();const container=root.querySelector('.terminal').getBoundingClientRect();return screen.right<=container.right && screen.width>=container.width-36})()");
    assert.equal(await run("singleDigitWidth<=30"),true,"single-digit block numbers use a compact gutter");
    await run("gutters()[0].click();gutters()[1].click();window.keptButton=gutters()[1];keptButton.focus()");
    await waitForValue(client,"context.length===2");
    assert.deepEqual(await run("context"),["one","two"]);
    await waitForValue(client,"root.querySelectorAll('.coterm-terminal-frame[data-selected=true]').length===2");
    assert.equal(await run("root.activeElement===keptButton"),true);
    await screenshot("ssh-blocks-dark");
    await run("document.documentElement.classList.remove('dark')");
    await screenshot("ssh-blocks-light");
    await run("document.documentElement.classList.add('dark');window.previousReads=calls.filter(c=>c.operation==='ssh.blocks').length;window.changeOnPage=true;for(let i=0;i<40;i++)event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"root.querySelector('.ssh-blocks-error').hidden && calls.filter(c=>c.operation==='ssh.blocks').length>=previousReads+5");
    assert.equal(await run("gutters()[1]===keptButton && root.activeElement===keptButton && keptButton.getAttribute('aria-pressed')==='true'"),true);
    await client.send("Input.dispatchKeyEvent",{type:"keyDown",key:"Escape",code:"Escape",windowsVirtualKeyCode:27});
    await client.send("Input.dispatchKeyEvent",{type:"keyUp",key:"Escape",code:"Escape",windowsVirtualKeyCode:27});
    await waitForValue(client,"context.length===0");
    await run("gutters()[0].click()");
    await waitForValue(client,"context.length===1");
    await run("root.querySelector('.xterm-helper-textarea').focus()");
    await client.send("Input.dispatchKeyEvent",{type:"keyDown",key:"Escape",code:"Escape",windowsVirtualKeyCode:27});
    await client.send("Input.dispatchKeyEvent",{type:"keyUp",key:"Escape",code:"Escape",windowsVirtualKeyCode:27});
    await waitForValue(client,"calls.some(c=>c.operation==='ssh.input')");
    assert.deepEqual(await run("context"),["one"],"terminal Esc does not clear block selection");
    assert.equal(await run("calls.filter(c=>c.operation==='ssh.input').every(c=>!atob(c.args.data).includes('Human')&&!atob(c.args.data).includes('Agent'))"),true);
    await run("root.querySelector('.job-block-number').click()");
    await waitForValue(client,"context.includes('job-3')");
    await run("root.querySelector('.coterm-terminal-clear').click()");
    await waitForValue(client,"context.length===0");
    await run("window.locale='zh-CN';window.translate()");
    assert.deepEqual(await run("(()=>{const b=root.querySelector('.coterm-terminal-clear');return [b.textContent,b.title,b.getAttribute('aria-label')]})()"),["×","取消全选","取消全选"]);
    await run("window.missingSelect=true;gutters()[0].click()");
    await waitForValue(client,"!root.querySelector('.ssh-blocks-error').hidden");
    assert.deepEqual(await run("[root.querySelector('.ssh-blocks-error .error').textContent,root.querySelector('.ssh-blocks-error button').hidden]"),["SSH 插件后端版本过旧，请重新构建并重启插件。",true]);
    await run("window.missingSelect=false;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"root.querySelector('.ssh-blocks-error').hidden && context.length===1");
    await run("root.querySelector('.coterm-terminal-clear').click()");
    await waitForValue(client,"context.length===0");
    await run("root.querySelector('.xterm-helper-textarea').focus();window.inputStart=calls.length");
    await client.send("Input.insertText",{text:"echo from-ui"});
    await client.send("Input.dispatchKeyEvent",{type:"keyDown",key:"Enter",code:"Enter",windowsVirtualKeyCode:13});
    await client.send("Input.dispatchKeyEvent",{type:"keyUp",key:"Enter",code:"Enter",windowsVirtualKeyCode:13});
    await waitForValue(client,"calls.slice(inputStart).filter(c=>c.operation==='ssh.input').map(c=>atob(c.args.data)).join('')==='echo from-ui\\r'");
    assert.equal(await run("calls.some(c=>c.operation==='ssh.command')"),false,"direct terminal input uses the existing PTY without an extra command submission");
    await run("backend.push(block('submitted',4,{command:'echo from-tool',actor:'robot',state:'running',exitCode:null}));version++;emit('',bound('submitted','begin'))");
    await waitForValue(client,"gutters().length===3");
    await run("emit('live reply\\r\\n');backend=backend.map(b=>b.id==='submitted'?{...b,state:'completed'}:b);version++;emit('',bound('submitted','end'));event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"gutters()[2].getAttribute('aria-label').includes('echo from-tool')");
    await run("gutters()[2].click();root.querySelector('.xterm-helper-textarea').focus()");
    // The source label is in the actual terminal buffer, searchable with its normal search UI.
    await client.send("Input.dispatchKeyEvent",{type:"keyDown",key:"f",code:"KeyF",modifiers:2,windowsVirtualKeyCode:70});
    await client.send("Input.dispatchKeyEvent",{type:"keyUp",key:"f",code:"KeyF",modifiers:2,windowsVirtualKeyCode:70});
    await waitForValue(client,"!root.querySelector('.terminal-find').hidden");
    await run("const search=root.querySelector('.terminal-find input');search.value='Agent';search.dispatchEvent(new Event('input',{bubbles:true}))");
    await waitForValue(client,"root.querySelector('.terminal-find-result').textContent.includes('2')");
    await run("root.querySelector('.terminal-find button:last-child').click()");
    for(const width of [768,320]){
      await client.send("Emulation.setDeviceMetricsOverride",{width,height:760,deviceScaleFactor:1,mobile:false});
      await waitForValue(client,"root.querySelector('.layout').scrollWidth<=innerWidth");
      await waitForValue(client,"(()=>{const screen=root.querySelector('.xterm-screen').getBoundingClientRect();const container=root.querySelector('.terminal').getBoundingClientRect();return screen.right<=container.right && screen.width>=container.width-36})()");
      assert.equal(await run("document.documentElement.scrollWidth<=innerWidth"),true);
    }
    await screenshot("ssh-blocks-narrow");
    await run("window.fail=true;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"!root.querySelector('.ssh-blocks-error').hidden");
    assert.equal(await run("gutters().length"),3,"failed read retains terminal and markers");
    await run("window.fail=false;backend=backend.filter(b=>b.id!=='submitted');window.dropped=1;version++;root.querySelector('.ssh-blocks-error button').click()");
    await waitForValue(client,"gutters().length===2 && context.length===0");
    await run("gutters()[0].click();emit('\\x1b[?1049h')");
    await waitForValue(client,"gutters().every(button=>button.hidden)");
    assert.deepEqual(await run("context"),["one"],"alternate screen does not destroy context selection");
    await run("emit('\\x1b[?1049l')");
    await waitForValue(client,"gutters().some(button=>!button.hidden)");
    await run("emit(('long output\\r\\n').repeat(3300))");
    await waitForValue(client,"gutters().length===0 && context.length===0");
    await run("window.epoch='transport-two';backend=[];version++;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"gutters().length===0");
    await run("window.delay=true;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client,"typeof window.release==='function'");
    await run("window.frontend.unmount();window.release();event({name:'ssh.blocks.changed',payload:{}})");
    assert.equal(await run("document.body.childElementCount"),0);
    assert.equal(await run("window.stopped&&window.localeStopped"),true);
    assert.deepEqual(errors,[]);
  }finally{await session.close();}
});
