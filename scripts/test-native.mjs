/** Real Windows WebView2 + Rust IPC acceptance. No host mocks; isolated app identifier. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectCdp, delay, evaluate, waitForValue } from "../tests/support/browser-harness.mjs";

if (process.platform !== "win32") throw new Error("Native acceptance requires Windows + WebView2");
const root = fileURLToPath(new URL("../", import.meta.url));
const executable = process.argv[2];
if (!executable) throw new Error("Usage: pnpm test:native <installed Coterm Smoke executable>");
const appData = path.join(process.env.APPDATA, "com.coterm.smoke");
// Never delete or modify an existing application workspace.
try { await fs.access(appData); throw new Error(`Smoke profile already exists: ${appData}. Move it aside before testing.`); }
catch (error) { if (error.code !== "ENOENT") throw error; }
const scratch = await fs.mkdtemp(path.join(os.tmpdir(), "coterm-native-"));
const packages = path.join(appData, "plugins", "fixture");
await fs.mkdir(path.join(packages, "backend"), { recursive: true });
await fs.mkdir(path.join(packages, "frontend"), { recursive: true });
await fs.copyFile(path.join(root, "target/debug/coterm-runtime-fixture.exe"), path.join(packages, "backend/fixture.exe"));
await fs.copyFile(path.join(root, "tests/fixtures/runtime-plugin/frontend.js"), path.join(packages, "frontend/index.js"));
const manifest = {
  manifestVersion: 1, id: "coterm-runtime-fixture", name: "Fixture", version: "0.1.0",
  runtime: { interfaceMajor: 1, requires: ["instances", "backend-calls", "operation-catalog", "secret-delivery", "deferred-interactions", "async-operations", "plugin-events"] },
  backend: { "windows-x64": "backend/fixture.exe", "windows-arm64": "backend/fixture.exe" },
  frontend: { entry: "frontend/index.js" }, mcp: { provider: "backend" },
  instances: { type: "test", multiple: true }, capabilities: ["secret-input"],
};
const manifestPath = path.join(packages, "manifest.json");
await fs.writeFile(manifestPath, JSON.stringify(manifest));
const fixturePids = new Set();
let app;
let client;
let failures = "";
async function launch() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => { const { port } = server.address(); server.close(() => resolve(port)); });
  });
  app = spawn(path.resolve(executable), [], { windowsHide: true, stdio: ["ignore", "ignore", "pipe"], env: {
    ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    WEBVIEW2_USER_DATA_FOLDER: path.join(scratch, "webview"),
  } });
  app.stderr.on("data", (chunk) => { failures += chunk.toString(); });
  const deadline = Date.now() + 30_000;
  let page;
  while (Date.now() < deadline) {
    if (app.exitCode !== null) throw new Error(`App exited ${app.exitCode}: ${failures}`);
    try {
      page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((item) => item.type === "page");
      if (page?.webSocketDebuggerUrl) break;
    } catch { /* WebView2 is starting. */ }
    await delay(100);
  }
  if (!page?.webSocketDebuggerUrl) throw new Error(`No native WebView CDP endpoint: ${failures}`);
  client = await connectCdp(page.webSocketDebuggerUrl);
  await client.send("Runtime.enable");
  await waitForValue(client, "!!window.__TAURI_INTERNALS__ && !!document.querySelector('[aria-label=\"Plugin list\"]')", { label: "real Tauri UI loaded" });
  await waitForValue(client, "!!document.querySelector('[aria-label=\"Manage Fixture\"]')", { label: "installed fixture discovered" });
}
const run = (expression) => evaluate(client, expression);
const invoke = (command, args = {}) => run(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
const operation = (id, name, waitTimeoutMs = 100) => invoke("invoke_operation", { pluginInstanceId: id, operation: name, args: {}, waitTimeoutMs });
async function poll(command, args, condition, label) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const result = await invoke(command, args);
    if (condition(result)) return result;
    await delay(50);
  }
  throw new Error(`Native condition timed out: ${label}`);
}
async function closeApp() {
  // Close through the actual Windows window message, exercising Tauri ExitRequested.
  const child = spawn("powershell", ["-NoProfile", "-NonInteractive", "-Command",
    `Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class CotermClose { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l); }'; $p = Get-Process -Id ${app.pid}; [CotermClose]::PostMessage($p.MainWindowHandle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null`],
  { windowsHide: true, stdio: "ignore" });
  await new Promise((resolve, reject) => { child.on("error", reject); child.on("exit", resolve); });
  const deadline = Date.now() + 10_000;
  while (app.exitCode === null && Date.now() < deadline) await delay(50);
  assert.equal(app.exitCode, 0, "normal Tauri exit finishes within 10 seconds");
  client.dispose();
  client = null;
  for (const pid of fixturePids) {
    let alive = true;
    try { process.kill(pid, 0); } catch { alive = false; }
    assert.equal(alive, false, `plugin child ${pid} was reaped on exit`);
  }
}
try {
  await launch();
  await invoke("start_plugin", { pluginId: manifest.id });
  const first = await invoke("create_instance", { pluginId: manifest.id, config: { label: "First", password: "native-config-secret" } });
  const second = await invoke("create_instance", { pluginId: manifest.id, config: { label: "Second" } });
  assert.notEqual(first.pluginInstanceId, second.pluginInstanceId);
  assert.notEqual(first.sessionId, second.sessionId);
  const info = await operation(first.pluginInstanceId, "fixture.info");
  fixturePids.add(info.output.pid);
  assert.equal((await operation(second.pluginInstanceId, "fixture.info")).output.instanceId, second.pluginInstanceId);
  await poll("runtime_snapshot", {}, (result) => result.instances.length === 2, "two instances");
  // Mount the real bundled plugin through the real workspace.
  await waitForValue(client, "!![...document.querySelectorAll('[aria-label=\"Instance list\"] button')].find(b=>b.textContent==='First')");
  await run("[...document.querySelectorAll('[aria-label=\"Instance list\"] button')].find(b=>b.textContent==='First').click()");
  await waitForValue(client, "!![...document.querySelectorAll('main button')].find(b=>b.textContent==='Text')");
  for (const [label, value, expected] of [["Text", "hello", "hello"], ["Selection", "b", "b"], ["Confirmation", null, true]]) {
    await run(`[...document.querySelectorAll('main button')].find(b=>b.textContent===${JSON.stringify(label)}).click()`);
    await waitForValue(client, "!!document.querySelector('dialog.host-secret-dialog[open]')");
    await run(`(() => { const d=document.querySelector('dialog.host-secret-dialog'); const input=d.querySelector('input,select'); if(input)input.value=${JSON.stringify(value)}; d.querySelector('form').requestSubmit(); })()`);
    await waitForValue(client, `document.querySelector('main output')?.textContent === ${JSON.stringify(JSON.stringify({ outcome: "submitted", value: expected }))}`);
  }
  // The WebView receives one unacknowledged event; a lagging consumer gets an explicit gap.
  await run("[...document.querySelectorAll('main button')].find(b=>b.textContent==='Hold events').click()");
  await waitForValue(client, "!!document.querySelector('main [data-subscribed=true]')");
  await operation(first.pluginInstanceId, "fixture.flood");
  await waitForValue(client, "JSON.parse(document.querySelector('main section').dataset.events || '[]').length === 1");
  await delay(500);
  assert.equal(await run("JSON.parse(document.querySelector('main section').dataset.events).length"), 1);
  await run("[...document.querySelectorAll('main button')].find(b=>b.textContent==='Resume events').click()");
  await waitForValue(client, "JSON.parse(document.querySelector('main section').dataset.events || '[]').includes('runtime.gap')");
  await operation(first.pluginInstanceId, "fixture.commit");
  const hanging = await operation(second.pluginInstanceId, "fixture.hang");
  await operation(first.pluginInstanceId, "fixture.crash").catch(() => {});
  await poll("runtime_snapshot", {}, (result) => result.instances.length === 2 && result.instances.every((instance) => instance.status === "failed"), "crashed instances converge");
  await waitForValue(client, "document.querySelector('main')?.textContent.includes('stopped unexpectedly')");
  assert.equal((await invoke("get_operation", { pluginInstanceId: second.pluginInstanceId, operationId: hanging.operationId })).status, "unknown");
  let saved = await invoke("workspace_snapshot");
  assert.equal(saved.entries.length, 2);
  assert.equal(JSON.stringify(saved).includes("native-config-secret"), false);
  assert.equal((await fs.readFile(path.join(appData, "workspace.v1.dpapi"))).includes(Buffer.from("native-config-secret")), false);
  const firstRef = saved.entries.find((entry) => entry.instanceId === first.pluginInstanceId)?.configRef;
  assert.ok(firstRef, "configuration reference is aligned to the original instance");
  const recovered = await invoke("restore_saved_instance", { configRef: firstRef });
  assert.notEqual(recovered.pluginInstanceId, first.pluginInstanceId);
  const restartedInfo = await operation(recovered.pluginInstanceId, "fixture.info");
  fixturePids.add(restartedInfo.output.pid);
  assert.equal(restartedInfo.output.writes, 0, "recovery does not replay writes");
  await waitForValue(client, "!![...document.querySelectorAll('[aria-label=\"Instance list\"] button')].find(b=>b.textContent==='First' && b.title.endsWith('ready'))");
  await run("[...document.querySelectorAll('[aria-label=\"Instance list\"] button')].find(b=>b.textContent==='First' && b.title.endsWith('ready')).click(); if(document.getElementById('toggle-instances-button').getAttribute('aria-expanded')==='true') document.getElementById('toggle-instances-button').click()");
  await poll("workspace_snapshot", {}, (result) => result.layout.instancesCollapsed, "UI layout persisted");
  await invoke("set_bypass", { enabled: true });
  await closeApp();
  console.log("PASS native multi-instance, SDK prompts, slow consumer/gap, crash/restart, encrypted persistence, exit reaping");

  // Relaunch headless: the same saved config reference creates a new instance on demand.
  delete manifest.frontend;
  await fs.writeFile(manifestPath, JSON.stringify(manifest));
  await launch();
  assert.equal((await invoke("list_instances")).length, 0, "startup does not auto execute or recreate connections");
  assert.equal(await invoke("get_bypass"), false, "Bypass never persists");
  saved = await invoke("workspace_snapshot");
  assert.equal(saved.layout.instancesCollapsed, true);
  assert.equal(saved.entries.length, 2);
  assert.ok(saved.entries.some((entry) => entry.configRef === firstRef), "stable configuration reference survives relaunch");
  const headless = await invoke("restore_saved_instance", { configRef: firstRef });
  assert.equal((await invoke("recent_operations", { pluginInstanceId: headless.pluginInstanceId, limit: 100 })).length, 0, "history is session-only");
  fixturePids.add((await operation(headless.pluginInstanceId, "fixture.info")).output.pid);
  const credential = await operation(headless.pluginInstanceId, "fixture.secret");
  assert.equal(credential.status, "running");
  await waitForValue(client, "document.querySelector('dialog h2')?.textContent === 'Headless fixture credential'");
  await run("document.querySelector('dialog input').value='native-entered-secret'; document.querySelector('dialog form').requestSubmit()");
  const final = await poll("get_operation", { pluginInstanceId: headless.pluginInstanceId, operationId: credential.operationId }, (record) => record.status === "completed", "headless secret completed");
  assert.deepEqual(final.output, { accepted: true });
  assert.equal(JSON.stringify(final).includes("native-entered-secret"), false);
  await operation(headless.pluginInstanceId, "fixture.block", 0);
  await delay(100);
  await closeApp();
  console.log("PASS native headless interaction, explicit restore, no write replay, session history, Bypass reset, blocked-control exit reaping");
} finally {
  if (app && app.exitCode === null) {
    try { await closeApp(); }
    catch {
      const killer = spawn("taskkill", ["/PID", String(app.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" });
      await new Promise((resolve) => killer.on("exit", resolve));
    }
  }
  client?.dispose();
  // Remove only the freshly-created, fixed smoke identifier and our own temp directory.
  for (let attempt = 0; attempt < 50; attempt++) {
    try { await fs.rm(scratch, { recursive: true, force: true }); break; }
    catch { await delay(100); }
  }
  for (let attempt = 0; attempt < 50; attempt++) {
    try { await fs.rm(appData, { recursive: true, force: true }); break; }
    catch (error) { if (attempt === 49) throw error; await delay(100); }
  }
}
