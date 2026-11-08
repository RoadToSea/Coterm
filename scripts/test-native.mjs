/** Windows WebView2 + bundled workers + real MCP Stdio, with an isolated smoke profile. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import {spawn} from "node:child_process";
import {createInterface} from "node:readline";
import {connectCdp,delay,evaluate,waitForValue,setAppLocale} from "../tests/support/browser-harness.mjs";

if(process.platform!=="win32")throw new Error("Requires Windows WebView2");
const executable=path.resolve(process.argv[2]??"target/debug/coterm-desktop.exe");
const profile=path.resolve(process.env.APPDATA,"com.coterm.smoke");
try{await fs.access(profile);throw new Error("Smoke profile already exists; refusing to overwrite it");}
catch(error){if(error.code!=="ENOENT")throw error;}
const scratch=await fs.mkdtemp(path.join(os.tmpdir(),"coterm-native-"));
const port=await new Promise((resolve,reject)=>{const server=net.createServer();server.on("error",reject);server.listen(0,"127.0.0.1",()=>{const port=server.address().port;server.close(()=>resolve(port));});});
let client,adapter;const calls=new Map();let sequence=0;
const app=spawn(executable,[],{windowsHide:true,stdio:["ignore","ignore","pipe"],env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(scratch,"webview")}});
let diagnostic="";app.stderr.on("data",data=>{diagnostic=(diagnostic+data).slice(-4096)});
const run=source=>evaluate(client,source);
const invoke=(command,args={})=>run(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)}).catch(reason=>{throw new Error(typeof reason==='string'?reason:JSON.stringify(reason))})`);
const operation=async(id,operation,args={})=>{const result=await invoke("invoke_operation",{driverInstanceId:id,operation,args,waitTimeoutMs:5000});if(result.error)throw new Error(JSON.stringify(result.error));return result.output;};
const rpc=(method,params={})=>new Promise((resolve,reject)=>{const id=++sequence;const timer=setTimeout(()=>{calls.delete(id);reject(new Error(`MCP ${method} timed out`))},15000);calls.set(id,{resolve,reject,timer});adapter.stdin.write(JSON.stringify({jsonrpc:"2.0",id,method,params})+"\n");});
const tool=(name,args)=>rpc("tools/call",{name,arguments:args});
async function approve(name,id,toolName,commands){
  const result=await tool(toolName,{name,commands});
  assert.equal(result.structuredContent.status,"pending_approval",JSON.stringify(result));
  await waitForValue(client,"!!document.querySelector('.approval-tray-shell button[type=submit]')");
  await run("document.querySelector('.approval-tray-shell button[type=submit]').click()");
  let record;
  for(let i=0;i<600;i++){record=await invoke("get_operation",{driverInstanceId:id,operationId:result.structuredContent.operationId});if(["completed","failed","unknown","cancelled"].includes(record.status))break;await delay(50);}
  assert.equal(record.status,"completed",JSON.stringify(record));
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
  assert.deepEqual(catalog.drivers.map(item=>item.id).sort(),["coterm-driver-local","coterm-driver-serial","coterm-driver-ssh"]);assert.deepEqual(catalog.issues,[]);
  await assert.rejects(invoke("prepare_driver_install",{locale:"en"}));
  await run("document.getElementById('open-settings').click()");await waitForValue(client,"document.getElementById('app-settings-dialog')?.open");
  assert.equal(await run("!!document.querySelector('[data-category=drivers]')"),false);
  await run("document.querySelector('#app-settings-dialog button[aria-label=\"Close dialog\"]').click()");
  await invoke("start_driver",{driverId:"coterm-driver-local"});
  const local=await invoke("create_instance",{driverId:"coterm-driver-local",config:{shell:"powershell.exe"},name:"Native local"});
  await waitForValue(client,`!!document.querySelector('[data-instance-id="${local.driverInstanceId}"]')`);
  await run(`document.querySelector('[data-instance-id="${local.driverInstanceId}"] > button').click()`);
  await waitForValue(client,"[...document.querySelectorAll('main div')].some(node=>node.shadowRoot?.querySelector('.xterm'))");
  for(let i=0;i<300;i++){if((await operation(local.driverInstanceId,"local.status")).running)break;await delay(50);}
  assert.equal((await operation(local.driverInstanceId,"local.status")).running,true);
  const mcp=await invoke("mcp_stdio_status");assert.equal(mcp.enabled,true);
  adapter=spawn(executable,["mcp","stdio","--addr",mcp.address],{windowsHide:true,stdio:["pipe","pipe","pipe"]});
  adapter.stderr.on("data",data=>{diagnostic=(diagnostic+data).slice(-4096)});
  createInterface({input:adapter.stdout}).on("line",line=>{const message=JSON.parse(line),call=calls.get(message.id);if(!call)return;calls.delete(message.id);clearTimeout(call.timer);message.error?call.reject(new Error(JSON.stringify(message.error))):call.resolve(message.result);});
  await rpc("initialize",{protocolVersion:"2024-11-05",capabilities:{},clientInfo:{name:"native-acceptance",version:"1"}});
  adapter.stdin.write(JSON.stringify({jsonrpc:"2.0",method:"notifications/initialized"})+"\n");
  const tools=(await rpc("tools/list")).tools.map(tool=>tool.name);assert.ok(tools.includes("coterm_local_command"));assert.ok(!tools.includes("coterm_local_exec"));
  await approve("Native local",local.driverInstanceId,"coterm_local_command",["$env:COTERM_ACCEPTANCE='retained'; Write-Output 'native-local-first'"]);
  await approve("Native local",local.driverInstanceId,"coterm_local_command",["Write-Output $env:COTERM_ACCEPTANCE"]);
  const pairs=await tool("coterm_local_read_dialogue_pairs",{name:"Native local",count:2});assert.equal(pairs.structuredContent.pairs.length,2);assert.ok(pairs.structuredContent.pairs[0].stdout.includes("retained"));
  if(process.env.COTERM_SERIAL_SHELL_LIVE){
    await invoke("start_driver",{driverId:"coterm-driver-serial"});
    const serial=await invoke("create_instance",{driverId:"coterm-driver-serial",name:"Native board",config:{port:process.env.COTERM_SERIAL_SHELL_LIVE,baudRate:Number(process.env.COTERM_SERIAL_LIVE_BAUD??115200),dtr:false,rts:false,mode:"stream"}});
    const config=(await operation(serial.driverInstanceId,"serial.configure",{mode:"shell",prompt:process.env.COTERM_SERIAL_SHELL_PROMPT??"# "})).config;
    await invoke("save_instance_configuration",{driverInstanceId:serial.driverInstanceId,config});await operation(serial.driverInstanceId,"serial.connect");
    try{
      await operation(serial.driverInstanceId,"serial.write",{text:"\r"});let ready=false;
      for(let i=0;i<100;i++){ready=(await operation(serial.driverInstanceId,"serial.status")).shellReady;if(ready)break;await delay(50);}assert.equal(ready,true,"Board must already be logged in at its prompt");
      await approve("Native board",serial.driverInstanceId,"coterm_serial_command",["printf '__COTERM_FULL_CHAIN__\\n'"]);
      const data=await tool("coterm_serial_read_dialogue_pairs",{name:"Native board",count:1});assert.ok(data.structuredContent.pairs[0].stdout.includes("__COTERM_FULL_CHAIN__"));assert.equal(data.structuredContent.pairs[0].stderr,"");assert.equal(data.structuredContent.pairs[0].exitCode,null);
    }finally{await operation(serial.driverInstanceId,"serial.disconnect");}
    const entry=(await invoke("workspace_snapshot")).entries.find(entry=>entry.instanceId===serial.driverInstanceId);
    await invoke("stop_driver",{driverId:"coterm-driver-serial"});const restored=await invoke("restore_saved_instance",{configRef:entry.configRef});
    const status=await operation(restored.driverInstanceId,"serial.status");assert.equal(status.config.mode,"shell");assert.equal(status.config.prompt,config.prompt);assert.equal(status.connected,false);
    console.log("PASS physical Serial: MCP -> approval UI -> worker -> board -> backend reads; saved Shell restore without reconnect");
  }
  console.log("PASS native bundled discovery, no installer, MCP Stdio, approval UI and shared PowerShell state");
}finally{
  adapter?.kill();for(const call of calls.values()){clearTimeout(call.timer);call.reject(new Error("Acceptance ended"));}
  if(app.exitCode===null){
    const closer=spawn("powershell",["-NoProfile","-NonInteractive","-Command",`Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class CotermClose { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l); }'; $p=Get-Process -Id ${app.pid}; [CotermClose]::PostMessage($p.MainWindowHandle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null`],{windowsHide:true,stdio:"ignore"});
    await new Promise(resolve=>closer.on("exit",resolve));for(let i=0;i<200&&app.exitCode===null;i++)await delay(50);if(app.exitCode===null)app.kill();
  }
  client?.dispose();
  if(path.basename(profile)!=="com.coterm.smoke"||!scratch.startsWith(path.resolve(os.tmpdir())+path.sep))throw new Error("Unexpected cleanup path");
  for(const target of [scratch,profile])for(let i=0;i<50;i++){try{await fs.rm(target,{recursive:true,force:true});break;}catch(error){if(i===49)throw error;await delay(100);}}
}
