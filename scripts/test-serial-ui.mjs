/**
 * 串口插件真实 Tauri 界面验收（开发构建，加载 plugins/plugin-serial/dist）。
 * 只在本机存在可枚举串口时运行；不写入物理设备，除非显式给出 COTERM_SERIAL_LIVE。
 */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectCdp, delay, evaluate, waitForValue } from "../tests/support/browser-harness.mjs";

if (process.platform !== "win32") throw new Error("Serial UI acceptance requires Windows + WebView2");
const root = fileURLToPath(new URL("../", import.meta.url));
const executable = process.argv[2] ?? path.join(root, "target/debug/coterm-desktop.exe");
const appData = path.join(process.env.APPDATA, "com.coterm.app");
const port = process.env.COTERM_SERIAL_LIVE_PORT ?? "COM6";
const scratch = await fs.mkdtemp(path.join(os.tmpdir(), "coterm-serial-ui-"));
let app;
let client;
let failures = "";

async function launch() {
  const debugPort = await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => { const { port } = server.address(); server.close(() => resolve(port)); });
  });
  app = spawn(path.resolve(executable), [], {
    windowsHide: true,
    stdio: ["ignore", "ignore", "pipe"],
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort}`,
      WEBVIEW2_USER_DATA_FOLDER: path.join(scratch, "webview"),
    },
  });
  app.stderr.on("data", (chunk) => { failures += chunk.toString(); });
  const deadline = Date.now() + 30_000;
  let page;
  while (Date.now() < deadline) {
    if (app.exitCode !== null) throw new Error(`App exited ${app.exitCode}: ${failures}`);
    try {
      page = (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find((item) => item.type === "page");
      if (page?.webSocketDebuggerUrl) break;
    } catch { /* WebView2 is starting. */ }
    await delay(100);
  }
  if (!page?.webSocketDebuggerUrl) throw new Error(`No native WebView CDP endpoint: ${failures}`);
  client = await connectCdp(page.webSocketDebuggerUrl);
  await client.send("Runtime.enable");
  // 系统语言可能是中文，节点的 aria-label 会被翻译；改用稳定的 id 判断加载。
  await waitForValue(client, "!!window.__TAURI_INTERNALS__ && !!document.getElementById('locale-select')", { label: "real Tauri UI loaded" });
  // 验收断言使用英文文案，先显式切换到英文；随后再验证语言切换。
  await evaluate(client, `(() => { const select = document.getElementById('locale-select');
    select.value = 'en'; select.dispatchEvent(new Event('change', { bubbles: true })); })()`);
  await waitForValue(client, "!!document.querySelector('[aria-label=\"Plugin list\"]')", { label: "English UI applied" });
}

const run = (expression) => evaluate(client, expression);
const invoke = (command, args = {}) => run(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);

async function serialRoot() {
  // 串口前端挂在自己的 Shadow DOM 里，用模式按钮定位它的宿主节点。
  return "([...document.querySelectorAll('main div')].find(node => node.shadowRoot?.querySelector('.modes'))?.shadowRoot)";
}

await launch();
try {
  const plugins = await invoke("list_plugins");
  const serial = plugins.plugins.find((plugin) => plugin.id === "coterm-plugin-serial");
  assert.ok(serial, `serial plugin must be discovered: ${JSON.stringify(plugins)}`);

  await run(`document.querySelector('[aria-label="Manage ${serial.name}"]').click()`);
  await waitForValue(client, "[...document.querySelectorAll('dialog div')].some(node=>node.shadowRoot?.querySelector('form'))", { label: "serial configuration form" });
  await run(`(() => {
    const host = [...document.querySelectorAll('dialog div')].find(node=>node.shadowRoot?.querySelector('form'));
    const root = host.shadowRoot;
    root.querySelector('input[type=text]').value = ${JSON.stringify(port)};
    root.querySelector('form').requestSubmit();
  })()`);
  await waitForValue(client, `${await serialRoot()}?.querySelector('.modes')`, { label: "serial workspace mounted" });

  // 默认是终端模式，UART 面板隐藏。
  const initial = await run(`(() => { const r = ${await serialRoot()};
    return { modes: [...r.querySelectorAll('.modes button')].map(b => b.textContent),
             terminalHidden: r.querySelector('.panel').hidden,
             uartHidden: [...r.querySelectorAll('.panel')][1].hidden,
             state: r.querySelector('.bar .state').textContent }; })()`);
  assert.deepEqual(initial.modes, ["Terminal", "UART console"]);
  assert.equal(initial.terminalHidden, false, "terminal mode is the default");
  assert.equal(initial.uartHidden, true);
  assert.equal(initial.state, "Disconnected");
  console.log("PASS serial frontend mounts with terminal and UART console modes");

  // 切到 UART 调试台，检查视图切换、暂停与清空都不需要后端。
  await run(`[...${await serialRoot()}.querySelectorAll('.modes button')][1].click()`);
  const uart = await run(`(() => { const r = ${await serialRoot()};
    return { terminalHidden: [...r.querySelectorAll('.panel')][0].hidden,
             uartHidden: [...r.querySelectorAll('.panel')][1].hidden,
             sendHidden: r.querySelector('.send').hidden,
             view: [...r.querySelectorAll('.uart-bar button')].slice(0,2).map(b => b.textContent),
             stats: r.querySelector('.uart-bar .stats').textContent }; })()`);
  assert.equal(uart.terminalHidden, true);
  assert.equal(uart.uartHidden, false);
  assert.equal(uart.sendHidden, true, "the terminal send row is hidden in UART mode");
  assert.deepEqual(uart.view, ["ASCII", "HEX"]);
  assert.match(uart.stats, /RX 0 B · TX 0 B/);
  console.log("PASS UART console switches layout, view buttons and byte counters");

  // 真实连接开发板：连接后计数、信号按钮与发送按钮应可用。
  await run(`[...${await serialRoot()}.querySelectorAll('.bar button')].find(b => b.textContent === 'Connect').click()`);
  await waitForValue(client, `${await serialRoot()}?.querySelector('.bar .state').textContent === 'Connected'`, { timeoutMs: 15_000, label: "serial port connected" });

  const connected = await run(`(() => { const r = ${await serialRoot()};
    return { send: !r.querySelector('.uart-send .primary').disabled,
             dtr: [...r.querySelectorAll('.signals button')].map(b => b.textContent),
             ports: r.querySelector('.bar .state').textContent }; })()`);
  assert.equal(connected.send, true);
  assert.deepEqual(connected.dtr, ["DTR High", "RTS High"]);

  // 真实开发板是 Linux 控制台：未登录时先 root 登录；上一次会话可能仍停在 shell。
  const type = async (value) => {
    await run(`(() => { const r = ${await serialRoot()}; const area = r.querySelector('.uart-send textarea');
      area.value = ${JSON.stringify('')} + ${JSON.stringify(value)}; r.querySelector('.uart-send .primary').click(); })()`);
  };
  await type(String.fromCharCode(13));
  await waitForValue(client, `[${await serialRoot()}?.querySelector('.uart-output').textContent.includes('login:'),
    ${await serialRoot()}?.querySelector('.uart-output').textContent.includes('# ') === true,
    ${await serialRoot()}?.querySelector('.uart-output').textContent.includes('Password:')].some(Boolean)`,
  { timeoutMs: 15_000, label: "board console prompt" });
  const consoleText = async () => run(`${await serialRoot()}.querySelector('.uart-output').textContent`);
  if ((await consoleText()).includes('login:')) {
    await type(process.env.COTERM_SERIAL_USER ?? "root");
    await waitForValue(client, `${await serialRoot()}?.querySelector('.uart-output').textContent.includes('Password:')`, { timeoutMs: 15_000, label: "password prompt" });
    await type(process.env.COTERM_SERIAL_PASSWORD ?? "root");
  }
  await waitForValue(client, `${await serialRoot()}?.querySelector('.uart-output').textContent.includes('# ')`, { timeoutMs: 15_000, label: "shell prompt after login" });
  console.log("PASS serial login interaction reached the shell prompt");

  // 发一条真实命令，验证文本通道、接收视图与字节计数。
  await type("uname -a");
  await waitForValue(client, `${await serialRoot()}?.querySelector('.uart-output').textContent.includes('Linux')`, { timeoutMs: 15_000, label: "board reply in UART console" });
  const received = await run(`(() => { const r = ${await serialRoot()};
    return { text: r.querySelector('.uart-output').textContent.slice(-400),
             stats: r.querySelector('.uart-bar .stats').textContent }; })()`);
  assert.match(received.text, /uname -a/);
  assert.match(received.text, /Linux \S+ \d+\./);
  assert.doesNotMatch(received.stats, /RX 0 B · TX 0 B/);
  // TX 计数必须包含插件补上的行结束符，不能只算输入文本长度。
  const counters = /RX (\d+) B · TX (\d+) B/.exec(received.stats);
  assert.ok(counters, `byte counters must be rendered: ${received.stats}`);
  const sent = "uname -a".length + 1; // 文本长度加一个 CR 行结束符
  assert.ok(Number(counters[2]) >= sent, `TX must count the line ending too: ${received.stats}`);
  console.log(`PASS real board round-trip in UART console (${received.stats.replace(/\s+/g, " ")})`);

  // HEX 视图必须给出十六进制与 ASCII 两列。
  await run(`[...${await serialRoot()}.querySelectorAll('.uart-bar button')].find(b => b.textContent === 'HEX').click()`);
  const hex = await run(`${await serialRoot()}.querySelector('.uart-output').textContent`);
  assert.match(hex, /[0-9a-f]{8}  [0-9a-f]{2} /, "HEX view renders an offset and byte columns");
  await run(`[...${await serialRoot()}.querySelectorAll('.uart-bar button')].find(b => b.textContent === 'ASCII').click()`);
  console.log("PASS HEX view renders offsets and byte columns");

  // 信号按钮走 serial.signals；语言切换不重建实例。
  await run(`[...${await serialRoot()}.querySelectorAll('.signals button')][0].click()`);
  await waitForValue(client, `${await serialRoot()}?.querySelector('.signals button').textContent === 'DTR Low'`, { label: "DTR lowered" });
  await run(`[...${await serialRoot()}.querySelectorAll('.signals button')][0].click()`);
  await waitForValue(client, `${await serialRoot()}?.querySelector('.signals button').textContent === 'DTR High'`, { label: "DTR restored" });
  console.log("PASS DTR/RTS controls drive serial.signals");

  await run(`(() => { const select=document.getElementById('locale-select'); select.value='zh-CN'; select.dispatchEvent(new Event('change',{bubbles:true})); })()`);
  await waitForValue(client, `${await serialRoot()}?.querySelector('.modes button').textContent === '终端'`, { label: "serial UI follows locale" });
  const zh = await run(`(() => { const r = ${await serialRoot()};
    return { modes: [...r.querySelectorAll('.modes button')].map(b => b.textContent),
             state: r.querySelector('.bar .state').textContent,
             keep: r.querySelector('.uart-output').textContent.includes('Linux') }; })()`);
  assert.deepEqual(zh.modes, ["终端", "UART 调试"]);
  assert.equal(zh.state, "已连接");
  assert.equal(zh.keep, true, "switching language keeps captured output");
  console.log("PASS serial frontend follows the host language without reconnecting");

  // 切回终端模式后断开，验证连接收尾。
  await run(`[...${await serialRoot()}.querySelectorAll('.modes button')][0].click()`);
  await run(`[...${await serialRoot()}.querySelectorAll('.bar button')].find(b => b.textContent === '断开连接').click()`);
  await waitForValue(client, `${await serialRoot()}?.querySelector('.bar .state').textContent === '未连接'`, { label: "serial port disconnected" });
  console.log("PASS disconnect returns the instance to a clean disconnected state");
} finally {
  try { await run("window.close()"); } catch { /* the page may already be closing. */ }
  await delay(500);
  if (app && app.exitCode === null) app.kill();
  await fs.rm(scratch, { recursive: true, force: true }).catch(() => {});
  if (failures.trim()) console.error(failures.slice(-2000));
}
console.log(`Serial UI acceptance used ${appData} and port ${port}`);
