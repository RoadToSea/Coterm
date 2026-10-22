/**
 * 浏览器用例的共享支架：静态服务当前构建 + 无头浏览器 + CDP 会话。
 *
 * 约束（对应本任务验收）：
 * - 只用当前构建：dist 缺失或比源码旧就失败，不隐式构建旧产物。
 * - 不冲突：调试端口由系统分配，浏览器 profile 落在独立临时目录，退出即删。
 * - 有限等待：所有 CDP 等待都带总超时，超时抛错而不是挂住。
 * - 无浏览器时明确 skip，不把 skip 当 pass（由调用方决定）。
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { spawn } from "node:child_process";

import { fileURLToPath } from "node:url";

export const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const distDir = path.join(rootDir, "apps/desktop/dist");

const BROWSER_CANDIDATES = [
  process.env.CHROME_BIN,
  process.env.EDGE_BIN,
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium-browser",
  "/usr/bin/chromium",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
].filter(Boolean);

export function findBrowser() {
  return BROWSER_CANDIDATES.find((candidate) => fs.existsSync(candidate)) ?? null;
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".mjs": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".json": "application/json",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
};

/** 构建产物必须比 apps/desktop 下任何源文件都新，否则是在测旧 dist。 */
export function assertFreshBuild() {
  const entry = path.join(distDir, "index.html");
  if (!fs.existsSync(entry)) {
    throw new Error(`apps/desktop/dist is missing; run "pnpm build" before the browser suite`);
  }
  const sshDist = path.join(rootDir, "plugins/plugin-ssh/dist/frontend/index.js");
  if (!fs.existsSync(sshDist)) {
    throw new Error(`plugins/plugin-ssh/dist/frontend/index.js is missing; run "pnpm --filter coterm-plugin-ssh-ui build"`);
  }
  const builtAt = fs.statSync(entry).mtimeMs;
  const stale = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === "dist" || entry.name === "node_modules") continue;
        walk(full);
      } else if (fs.statSync(full).mtimeMs > builtAt) {
        stale.push(path.relative(rootDir, full));
      }
    }
  };
  walk(path.join(rootDir, "apps/desktop/src"));
  walk(path.join(rootDir, "packages/plugin-sdk/src"));
  if (stale.length > 0) {
    throw new Error(`dist is older than these sources; rebuild before testing: ${stale.slice(0, 5).join(", ")}`);
  }
}

export async function startStaticServer() {
  const server = http.createServer((request, response) => {
    const requestPath = decodeURIComponent((request.url ?? "/").split("?")[0]) || "/";
    const relative = requestPath === "/" ? "index.html" : requestPath.replace(/^\/+/, "");
    const filePath = path.join(distDir, relative);
    if (!filePath.startsWith(distDir) || !fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
      response.writeHead(404, { "Content-Type": "text/plain" });
      response.end("Not Found");
      return;
    }
    response.writeHead(200, { "Content-Type": MIME[path.extname(filePath)] ?? "application/octet-stream" });
    fs.createReadStream(filePath).pipe(response);
  });
  const port = await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve(server.address().port));
  });
  return { server, port, url: `http://127.0.0.1:${port}/` };
}

async function freePort() {
  const probe = http.createServer();
  const port = await new Promise((resolve, reject) => {
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => resolve(probe.address().port));
  });
  await new Promise((resolve) => probe.close(resolve));
  return port;
}

function withTimeout(promise, ms, label) {
  let timer;
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`Timed out after ${ms}ms: ${label}`)), ms);
    }),
  ]).finally(() => clearTimeout(timer));
}

/**
 * 打开一个无头浏览器会话。调用方必须在 finally 里 await session.close()。
 * totalTimeoutMs 覆盖整个用例，超时后浏览器被强杀，避免用例挂住。
 */
export async function openSession({ browserPath, totalTimeoutMs = 90_000 }) {
  const debugPort = await freePort();
  const profileDir = fs.mkdtempSync(path.join(os.tmpdir(), "coterm-layout-"));
  const startedAt = Date.now();
  const browser = spawn(browserPath, [
    "--headless=new",
    `--remote-debugging-port=${debugPort}`,
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-gpu",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-sync",
    "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });

  const cleanup = async () => {
    if (!browser.killed) browser.kill("SIGKILL");
    for (let attempt = 0; attempt < 50; attempt++) {
      if (!fs.existsSync(profileDir)) break;
      try {
        fs.rmSync(profileDir, { recursive: true, force: true });
        break;
      } catch {
        await delay(100);
      }
    }
  };

  try {
    const wsUrl = await withTimeout((async () => {
      for (let attempt = 0; attempt < 60; attempt++) {
        if (browser.exitCode !== null) throw new Error(`Headless browser exited early with code ${browser.exitCode}`);
        try {
          const list = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
          const page = list.find((item) => item.type === "page");
          if (page?.webSocketDebuggerUrl) return page.webSocketDebuggerUrl;
        } catch {
          // 浏览器还没开始监听调试端口，继续等。
        }
        await delay(200);
      }
      throw new Error("Headless browser never exposed a CDP WebSocket URL");
    })(), 20_000, "wait for CDP endpoint");

    const client = await withTimeout(connectCdp(wsUrl, () => Date.now() - startedAt > totalTimeoutMs), 15_000, "connect CDP");
    return {
      client,
      async close() {
        client.dispose();
        await cleanup();
      },
    };
  } catch (error) {
    await cleanup();
    throw error;
  }
}

function connectCdp(wsUrl, expired) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(wsUrl);
    const pending = new Map();
    // 事件按 sessionId 分桶：flatten 模式下同一条 socket 承载多个页面。
    const listeners = new Map();
    let requestId = 1;
    let closed = false;

    const eventKey = (sessionId, method) => `${sessionId ?? "root"}::${method}`;

    const failAll = (error) => {
      for (const { reject: rejectRequest } of pending.values()) rejectRequest(error);
      pending.clear();
    };

    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.method) {
        for (const handler of listeners.get(eventKey(message.sessionId, message.method)) ?? []) handler(message.params);
        return;
      }
      const waiter = pending.get(message.id);
      if (!waiter) return;
      pending.delete(message.id);
      if (message.error) waiter.reject(new Error(`${waiter.method}: ${JSON.stringify(message.error)}`));
      else waiter.resolve(message.result);
    };
    socket.onerror = () => {
      closed = true;
      failAll(new Error("CDP socket error"));
    };
    socket.onclose = () => {
      closed = true;
      failAll(new Error("CDP socket closed"));
    };

    const send = (method, params = {}, sessionId) => {
      if (closed) return Promise.reject(new Error(`CDP socket closed before ${method}`));
      if (expired()) return Promise.reject(new Error(`Case timeout reached before ${method}`));
      const id = requestId++;
      socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
      return withTimeout(new Promise((resolveRequest, rejectRequest) => {
        pending.set(id, { resolve: resolveRequest, reject: rejectRequest, method });
      }), 20_000, method);
    };

    const waitFor = (method, timeoutMs = 20_000, sessionId) => withTimeout(
      new Promise((resolveEvent) => {
        const key = eventKey(sessionId, method);
        const handler = (params) => {
          listeners.set(key, (listeners.get(key) ?? []).filter((item) => item !== handler));
          resolveEvent(params);
        };
        listeners.set(key, [...(listeners.get(key) ?? []), handler]);
      }),
      timeoutMs,
      `event ${method}`,
    );

    const on = (method, handler, sessionId) => {
      const key = eventKey(sessionId, method);
      listeners.set(key, [...(listeners.get(key) ?? []), handler]);
      return () => {
        listeners.set(key, (listeners.get(key) ?? []).filter((item) => item !== handler));
      };
    };

    socket.onopen = () => resolve({
      send,
      waitFor,
      on,
      dispose() {
        closed = true;
        try { socket.close(); } catch { /* 已经关闭 */ }
      },
    });
    socket.onerror = reject;
  });
}

export const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** 轮询求值直到返回真值；超时抛错，避免 flaky 的固定 sleep。 */
export async function waitForValue(client, expression, { timeoutMs = 10_000, intervalMs = 50, label = expression } = {}) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const { result } = await client.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (result?.value) return result.value;
    await delay(intervalMs);
  }
  throw new Error(`Condition never held within ${timeoutMs}ms: ${label}`);
}

export async function evaluate(client, expression) {
  const { result, exceptionDetails } = await client.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (exceptionDetails) {
    throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
  }
  return result?.value;
}
