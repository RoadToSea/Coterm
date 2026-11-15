/** Windows WebView2 + in-process drivers + real MCP HTTP, with an isolated smoke profile. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import {spawn} from "node:child_process";
import {connectCdp,delay,evaluate,waitForValue,setAppLocale} from "../tests/support/browser-harness.mjs";

if(process.platform!=="win32")throw new Error("Requires Windows WebView2");
const executable=path.resolve(process.argv[2]??"target/debug/coterm-desktop.exe");
const profile=path.resolve(process.env.APPDATA,"com.coterm.smoke");
try{await fs.access(profile);throw new Error("Smoke profile already exists; refusing to overwrite it");}
catch(error){if(error.code!=="ENOENT")throw error;}
const scratch=await fs.mkdtemp(path.join(os.tmpdir(),"coterm-native-"));
const port=await new Promise((resolve,reject)=>{const server=net.createServer();server.on("error",reject);server.listen(0,"127.0.0.1",()=>{const port=server.address().port;server.close(()=>resolve(port));});});
let client,mcpUrl;let sequence=0;
const requests=new Set();
const networkPeers=new Set();const networkServers=[];
const app=spawn(executable,[],{windowsHide:true,stdio:["ignore","ignore","pipe"],env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(scratch,"webview")}});
let diagnostic="";app.stderr.on("data",data=>{diagnostic=(diagnostic+data).slice(-4096)});
const run=source=>evaluate(client,source);
const invoke=(command,args={})=>run(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)}).catch(reason=>{throw new Error(typeof reason==='string'?reason:JSON.stringify(reason))})`);
const operation=async(id,operation,args={})=>{const result=await invoke("invoke_operation",{driverInstanceId:id,operation,args,waitTimeoutMs:5000});if(result.error)throw new Error(JSON.stringify(result.error));return result.output;};
async function rpc(method,params={}){
  const id=++sequence,controller=new AbortController();requests.add(controller);
  try{
    const reply=await fetch(mcpUrl,{method:"POST",headers:{"Content-Type":"application/json",Accept:"application/json, text/event-stream","MCP-Protocol-Version":"2025-06-18"},body:JSON.stringify({jsonrpc:"2.0",id,method,params}),signal:AbortSignal.any([controller.signal,AbortSignal.timeout(15000)])});
    assert.equal(reply.status,200);const message=await reply.json();assert.equal(message.id,id);
    if(message.error)throw new Error(JSON.stringify(message.error));return message.result;
  }finally{requests.delete(controller);}
}
const tool=(name,args)=>rpc("tools/call",{name,arguments:args});
async function closeAppWindow(){
  const closer=spawn("powershell",["-NoProfile","-NonInteractive","-Command",`Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class CotermClose { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l); }'; $p=Get-Process -Id ${app.pid}; [CotermClose]::PostMessage($p.MainWindowHandle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null`],{windowsHide:true,stdio:"ignore"});
  await new Promise(resolve=>closer.on("exit",resolve));
}
async function expectExit(child,label,timeoutMs,codes=[0]){
  for(let elapsed=0;elapsed<timeoutMs&&child.exitCode===null&&child.signalCode===null;elapsed+=50)await delay(50);
  assert.ok(codes.includes(child.exitCode),`${label} must exit without forced termination; code=${child.exitCode}, signal=${child.signalCode}`);
}
function bounded(promise,message){
  let timer;return Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(message)),5000);})]).finally(()=>clearTimeout(timer));
}
async function idleConnection(url){
  const endpoint=new URL(url),socket=net.createConnection({host:endpoint.hostname,port:Number(endpoint.port)});
  networkPeers.add(socket);socket.on("close",()=>networkPeers.delete(socket));socket.on("error",()=>{});
  await bounded(new Promise((resolve,reject)=>{socket.once("connect",resolve);socket.once("error",reject);}),"HTTP client did not connect");
  const received=new Promise((resolve,reject)=>{socket.once("data",resolve);socket.once("error",reject);});
  socket.write(`GET /mcp HTTP/1.1\r\nHost: ${endpoint.host}\r\n\r\n`);
  await bounded(received,"HTTP endpoint did not answer GET");return socket;
}
const expectClosed=socket=>socket.closed?Promise.resolve():bounded(new Promise(resolve=>socket.once("close",resolve)),"HTTP connection survived service shutdown");
async function approve(name,id,toolName,commands){
  const result=await tool(toolName,{name,commands});
  assert.equal(result.structuredContent.status,"pending_approval",JSON.stringify(result));
  const editorId=`agent-approval-${result.structuredContent.operationId}`;
  await waitForValue(client,`!!document.getElementById(${JSON.stringify(editorId)})?.closest('form').querySelector('button[type=submit]:not(:disabled)')`);
  await run(`document.getElementById(${JSON.stringify(editorId)}).closest('form').querySelector('button[type=submit]').click()`);
  let record;
  for(let i=0;i<600;i++){record=await invoke("get_operation",{driverInstanceId:id,operationId:result.structuredContent.operationId});if(["completed","failed","unknown","cancelled"].includes(record.status))break;
    const error=await run(`document.getElementById(${JSON.stringify(editorId)})?.closest('form').querySelector('[role=alert]')?.textContent`);
    assert.ok(!error,error);await delay(50);}
  assert.equal(record.status,"completed",JSON.stringify(record));
  await waitForValue(client,`!document.getElementById(${JSON.stringify(editorId)})`);
}
try{
  let page;
  for(let i=0;i<300;i++){
    if(app.exitCode!==null)throw new Error(`Desktop exited: ${diagnostic}`);
    try{page=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(item=>item.type==="page");}catch{}
    if(page?.webSocketDebuggerUrl)break;await delay(100);
  }
  if(!page?.webSocketDebuggerUrl)throw new Error(`WebView2 unavailable: ${diagnostic}`);
  client=await connectCdp(page.webSocketDebuggerUrl);await client.send("Runtime.enable");
  await waitForValue(client,"!!document.getElementById('open-settings')");
  assert.equal(await invoke("plugin:app|name"),"Coterm Smoke","Build with tests/fixtures/tauri-smoke.conf.json");
  await setAppLocale(client,"en");
  const catalog=await invoke("list_drivers");
  assert.deepEqual(catalog.drivers.map(item=>item.id).sort(),["coterm-driver-local","coterm-driver-serial","coterm-driver-ssh","coterm-driver-tcp","coterm-driver-telnet"]);assert.deepEqual(catalog.issues,[]);
  await assert.rejects(invoke("prepare_driver_install",{locale:"en"}));
  await run("document.getElementById('open-settings').click()");await waitForValue(client,"document.getElementById('app-settings-dialog')?.open");
  assert.equal(await run("!!document.querySelector('[data-category=drivers]')"),false);
  await run("document.querySelector('#app-settings-dialog button[aria-label=\"Close dialog\"]').click()");
  await invoke("start_driver",{driverId:"coterm-driver-local"});
  const local=await invoke("create_instance",{driverId:"coterm-driver-local",config:{shell:"powershell.exe"},name:"Native local"});
  await waitForValue(client,`!!document.querySelector('[data-instance-id="${local.driverInstanceId}"]')`);
  await run(`document.querySelector('[data-instance-id="${local.driverInstanceId}"] > button').click()`);
  await waitForValue(client,"[...document.querySelectorAll('main div')].some(node=>node.querySelector('.xterm'))");
  for(let i=0;i<300;i++){if((await operation(local.driverInstanceId,"local.status")).running)break;await delay(50);}
  assert.equal((await operation(local.driverInstanceId,"local.status")).running,true);
  const mcp=await invoke("mcp_status");assert.equal(mcp.enabled,true);mcpUrl=mcp.url;
  assert.deepEqual(JSON.parse(mcp.clientConfig).mcpServers.coterm,{type:"http",url:mcpUrl});
  await run("document.getElementById('open-settings').click()");await waitForValue(client,"document.getElementById('app-settings-dialog')?.open");
  await run("document.querySelector('[data-category=mcp]').click()");await waitForValue(client,"!!document.querySelector('[data-mcp-config]:not(:disabled)')");
  await run("document.querySelector('[data-mcp-config]').click()");await waitForValue(client,"!!document.querySelector('textarea[aria-label=\"HTTP client configuration\"]')");
  assert.deepEqual(await run("JSON.parse(document.querySelector('textarea[aria-label=\"HTTP client configuration\"]').value).mcpServers.coterm"),{type:"http",url:mcpUrl});
  await run("document.querySelector('dialog[aria-label=\"MCP connection\"] header button').click();document.querySelector('#app-settings-dialog header button').click()");
  await rpc("initialize",{protocolVersion:"2025-06-18",capabilities:{},clientInfo:{name:"native-acceptance",version:"1"}});
  const initialized=await fetch(mcpUrl,{method:"POST",headers:{"Content-Type":"application/json",Accept:"application/json, text/event-stream"},body:JSON.stringify({jsonrpc:"2.0",method:"notifications/initialized"}),signal:AbortSignal.timeout(5000)});
  assert.equal(initialized.status,202);
  const tools=(await rpc("tools/list")).tools.map(tool=>tool.name);assert.equal(tools.length,8);assert.ok(tools.includes("coterm_terminal_command"));assert.ok(!tools.includes("coterm_local_command"));
  await approve("Native local",local.driverInstanceId,"coterm_terminal_command",["$env:COTERM_ACCEPTANCE='retained'; Write-Output 'native-local-first'"]);
  await approve("Native local",local.driverInstanceId,"coterm_terminal_command",["Write-Output $env:COTERM_ACCEPTANCE"]);
  const pairs=await tool("coterm_read_output",{name:"Native local",mode:"dialogue",count:2});assert.equal(pairs.structuredContent.pairs.length,2);assert.ok(pairs.structuredContent.pairs[0].stdout.includes("retained"));
  for(const protocol of ["tcp","telnet"]){
    const server=net.createServer(socket=>{
      networkPeers.add(socket);socket.on("close",()=>networkPeers.delete(socket));socket.on("error",()=>{});
      socket.write("test# ");let pending="";
      socket.on("data",data=>{pending+=data.toString("utf8");if(pending.length>16384){socket.destroy();return;}
        let end;while((end=pending.indexOf("\n"))>=0){const command=pending.slice(0,end).replace(/\r$/u,"");pending=pending.slice(end+1);socket.write(`${command}\r\n${protocol}-native-result\r\ntest# `);}});
    });networkServers.push(server);
    await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
    const driverId=`coterm-driver-${protocol}`,name=`Native ${protocol}`;
    await invoke("start_driver",{driverId});
    const instance=await invoke("create_instance",{driverId,name,config:{host:"127.0.0.1",port:server.address().port,mode:"shell",prompt:"test# ",localEcho:"off",backspace:"del",autoReconnect:false}});
    assert.equal((await operation(instance.driverInstanceId,`${protocol}.status`)).connected,false);
    await operation(instance.driverInstanceId,`${protocol}.connect`);
    for(let attempt=0;attempt<100;attempt++){if((await operation(instance.driverInstanceId,`${protocol}.status`)).shellReady)break;await delay(20);}
    const networkTools=(await rpc("tools/list")).tools.map(tool=>tool.name);
    assert.ok(networkTools.includes("coterm_stream_command"));assert.ok(!networkTools.includes(`coterm_${protocol}_command`));
    await approve(name,instance.driverInstanceId,"coterm_stream_command",["native-loopback"]);
    const result=await tool("coterm_read_output",{name,mode:"dialogue",count:1});
    assert.ok(result.structuredContent.pairs[0].stdout.includes(`${protocol}-native-result`));
    assert.equal(result.structuredContent.pairs[0].stderr,"");assert.equal(result.structuredContent.pairs[0].exitCode,null);
    const [localReply,networkReply]=await Promise.all([tool("coterm_read_output",{name:"Native local",mode:"dialogue",count:1}),tool("coterm_read_output",{name,mode:"dialogue",count:1})]);
    assert.ok(localReply.structuredContent.pairs[0].stdout.includes("retained"));assert.ok(networkReply.structuredContent.pairs[0].stdout.includes(`${protocol}-native-result`));
    const entry=(await invoke("workspace_snapshot")).entries.find(entry=>entry.instanceId===instance.driverInstanceId);
    await invoke("stop_driver",{driverId});const restored=await invoke("restore_saved_instance",{configRef:entry.configRef});
    const status=await operation(restored.driverInstanceId,`${protocol}.status`);
    assert.equal(status.connected,false);assert.equal(status.config.mode,"shell");assert.equal(status.config.backspace,"del");
    console.log(`PASS ${protocol}: real MCP approval, loopback command/results and saved restore without reconnect`);
  }
  if(process.env.COTERM_SERIAL_SHELL_LIVE){
    await invoke("start_driver",{driverId:"coterm-driver-serial"});
    const serial=await invoke("create_instance",{driverId:"coterm-driver-serial",name:"Native board",config:{port:process.env.COTERM_SERIAL_SHELL_LIVE,baudRate:Number(process.env.COTERM_SERIAL_LIVE_BAUD??115200),dtr:false,rts:false,mode:"stream"}});
    const config=(await operation(serial.driverInstanceId,"serial.configure",{mode:"shell",prompt:process.env.COTERM_SERIAL_SHELL_PROMPT??"# "})).config;
    await invoke("save_instance_configuration",{driverInstanceId:serial.driverInstanceId,config});await operation(serial.driverInstanceId,"serial.connect");
    try{
      await operation(serial.driverInstanceId,"serial.write",{text:"\r"});let ready=false;
      for(let i=0;i<100;i++){ready=(await operation(serial.driverInstanceId,"serial.status")).shellReady;if(ready)break;await delay(50);}assert.equal(ready,true,"Board must already be logged in at its prompt");
      await approve("Native board",serial.driverInstanceId,"coterm_stream_command",["printf '__COTERM_FULL_CHAIN__\\n'"]);
      const data=await tool("coterm_read_output",{name:"Native board",mode:"dialogue",count:1});assert.ok(data.structuredContent.pairs[0].stdout.includes("__COTERM_FULL_CHAIN__"));assert.equal(data.structuredContent.pairs[0].stderr,"");assert.equal(data.structuredContent.pairs[0].exitCode,null);
    }finally{await operation(serial.driverInstanceId,"serial.disconnect");}
    const entry=(await invoke("workspace_snapshot")).entries.find(entry=>entry.instanceId===serial.driverInstanceId);
    await invoke("stop_driver",{driverId:"coterm-driver-serial"});const restored=await invoke("restore_saved_instance",{configRef:entry.configRef});
    const status=await operation(restored.driverInstanceId,"serial.status");assert.equal(status.config.mode,"shell");assert.equal(status.config.prompt,config.prompt);assert.equal(status.connected,false);
    console.log("PASS physical Serial: MCP -> approval UI -> driver -> board -> backend reads; saved Shell restore without reconnect");
  }
  console.log("PASS native in-process drivers, MCP HTTP, approval UI and shared PowerShell state");
  const oldIdle=await idleConnection(mcpUrl),newPort=net.createServer();
  await new Promise(resolve=>newPort.listen(0,"127.0.0.1",resolve));const nextPort=newPort.address().port;await new Promise(resolve=>newPort.close(resolve));
  const updated=await invoke("set_mcp_port",{request:{port:nextPort}});await expectClosed(oldIdle);mcpUrl=updated.url;
  assert.equal(mcpUrl,`http://127.0.0.1:${nextPort}/mcp`);assert.equal((await rpc("tools/list")).tools.length,8);
  const savedMcp=JSON.parse(await fs.readFile(path.join(profile,"mcp-settings.v1.json"),"utf8"));assert.equal(savedMcp.version,1);assert.equal(savedMcp.port,nextPort);
  const stoppedIdle=await idleConnection(mcpUrl);await invoke("set_mcp_enabled",{enabled:false});await expectClosed(stoppedIdle);
  await assert.rejects(fetch(mcpUrl,{signal:AbortSignal.timeout(3000)}));
  const restarted=await invoke("set_mcp_enabled",{enabled:true});assert.equal(restarted.url,mcpUrl);assert.equal((await rpc("tools/list")).tools.length,8);
  console.log("PASS HTTP port changes, stop/start, unchanged settings format and independent concurrent instance results");
  const endpoint=new URL(mcpUrl),idle=await idleConnection(mcpUrl);
  await closeAppWindow();
  await expectExit(app,"Desktop after closing its window",10000);
  await expectClosed(idle);
  const reservation=net.createServer();await new Promise((resolve,reject)=>{reservation.once("error",reject);reservation.listen(Number(endpoint.port),endpoint.hostname,resolve);});
  await new Promise(resolve=>reservation.close(resolve));
  console.log("PASS window close releases the desktop, idle HTTP connections and MCP port without an adapter process");
}finally{
  for(const socket of networkPeers)socket.destroy();
  for(const server of networkServers)if(server.listening)await new Promise(resolve=>server.close(resolve));
  for(const controller of requests)controller.abort();
  if(app.exitCode===null){
    await closeAppWindow();for(let i=0;i<200&&app.exitCode===null;i++)await delay(50);if(app.exitCode===null)app.kill();
  }
  client?.dispose();
  if(path.basename(profile)!=="com.coterm.smoke"||!scratch.startsWith(path.resolve(os.tmpdir())+path.sep))throw new Error("Unexpected cleanup path");
  for(const target of [scratch,profile])for(let i=0;i<50;i++){try{await fs.rm(target,{recursive:true,force:true});break;}catch(error){if(i===49)throw error;await delay(100);}}
}
