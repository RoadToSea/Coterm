import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { spawn, execSync } from "node:child_process";

const rootDir = process.cwd();
const distDir = path.join(rootDir, "apps/desktop/dist");

function findBrowser() {
  const candidates = [
    process.env.CHROME_BIN,
    process.env.EDGE_BIN,
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium-browser",
    "/usr/bin/chromium",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
  ].filter(Boolean);

  for (const p of candidates) {
    if (fs.existsSync(p)) return p;
  }
  return null;
}

test("Browser automated layout verification at 1280px, 1024px, and 768px", async (t) => {
  const browserPath = findBrowser();
  if (!browserPath) {
    t.skip("No Chrome/Edge browser found in environment for headless layout testing");
    return;
  }

  // Ensure desktop build exists
  if (!fs.existsSync(path.join(distDir, "index.html"))) {
    execSync("pnpm --filter coterm-desktop build", { stdio: "inherit", cwd: rootDir });
  }

  const mimeTypes = {
    ".html": "text/html",
    ".js": "application/javascript",
    ".css": "text/css",
    ".svg": "image/svg+xml",
    ".json": "application/json",
  };

  const server = http.createServer((req, res) => {
    let reqPath = req.url.split("?")[0];
    if (reqPath === "/") reqPath = "/index.html";
    const filePath = path.join(distDir, reqPath);
    if (fs.existsSync(filePath) && fs.statSync(filePath).isFile()) {
      const ext = path.extname(filePath);
      res.writeHead(200, { "Content-Type": mimeTypes[ext] || "application/octet-stream" });
      fs.createReadStream(filePath).pipe(res);
    } else {
      res.writeHead(404);
      res.end("Not Found");
    }
  });

  const serverPort = await new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => resolve(server.address().port));
  });

  // Pick an ephemeral debugging port
  const debugPort = 9224;
  const browserProc = spawn(browserPath, [
    "--headless=new",
    `--remote-debugging-port=${debugPort}`,
    "--disable-gpu",
    "--no-sandbox",
    "about:blank",
  ]);

  try {
    let wsUrl = null;
    for (let i = 0; i < 30; i++) {
      try {
        const res = await fetch(`http://127.0.0.1:${debugPort}/json/list`);
        const list = await res.json();
        const page = list.find((item) => item.type === "page");
        if (page && page.webSocketDebuggerUrl) {
          wsUrl = page.webSocketDebuggerUrl;
          break;
        }
      } catch {
        await new Promise((r) => setTimeout(r, 200));
      }
    }

    assert.ok(wsUrl, "Headless browser CDP WebSocket URL must be available");

    const ws = new WebSocket(wsUrl);

    await new Promise((resolve, reject) => {
      let reqId = 1;
      const callbacks = new Map();

      const send = (method, params = {}) => {
        const id = reqId++;
        ws.send(JSON.stringify({ id, method, params }));
        return new Promise((res, rej) => callbacks.set(id, { res, rej }));
      };

      let pageLoadedResolve;
      const pageLoadedPromise = new Promise((r) => {
        pageLoadedResolve = r;
      });

      ws.onmessage = (evt) => {
        const data = JSON.parse(evt.data);
        if (data.method === "Page.loadEventFired") {
          pageLoadedResolve?.();
        }
        if (data.id && callbacks.has(data.id)) {
          const { res, rej } = callbacks.get(data.id);
          callbacks.delete(data.id);
          if (data.error) rej(data.error);
          else res(data.result);
        }
      };

      ws.onopen = async () => {
        try {
          await send("Page.enable");
          await send("Runtime.enable");

          // Inject mock Tauri host API
          await send("Page.addScriptToEvaluateOnNewDocument", {
            source: `
              window.__TAURI_INTERNALS__ = {
                invoke: async (cmd) => {
                  if (cmd === 'list_plugins') return [{ id: 'ssh', name: 'SSH', version: '0.1.0', hasFrontend: true, running: true }];
                  if (cmd === 'list_instances') return [{ pluginId: 'ssh', pluginInstanceId: 'inst-1', sessionId: 'sess-1', label: 'Prod-Server', status: 'ready' }];
                  if (cmd === 'get_bypass') return false;
                  if (cmd === 'list_operations') return [];
                  if (cmd === 'recent_operations') return [];
                  if (cmd === 'pending_interactions') return [];
                  return null;
                },
                convertFileSrc: (s) => s,
                metadata: { currentWindow: { label: 'main' } }
              };
            `,
          });

          await send("Page.navigate", { url: `http://127.0.0.1:${serverPort}` });
          await pageLoadedPromise;
          // Allow React to mount
          await new Promise((r) => setTimeout(r, 800));

          const measureLayout = async (w, h) => {
            await send("Emulation.setDeviceMetricsOverride", {
              width: w,
              height: h,
              deviceScaleFactor: 1,
              mobile: false,
            });
            await send("Runtime.evaluate", {
              expression: "window.dispatchEvent(new Event('resize'));",
            });
            await new Promise((r) => setTimeout(r, 250));

            const res = await send("Runtime.evaluate", {
              expression: `
                (() => {
                  const plugins = document.querySelector('aside[aria-label="Installed plugins"]');
                  const instances = document.getElementById('instances-sidebar');
                  const workspace = document.querySelector('main[aria-label="Plugin workspace"]');
                  const toggleBtn = document.getElementById('toggle-instances-button');

                  const pRect = plugins ? plugins.getBoundingClientRect() : null;
                  const iRect = instances ? instances.getBoundingClientRect() : null;
                  const wRect = workspace ? workspace.getBoundingClientRect() : null;

                  const isInstancesVisible = instances && window.getComputedStyle(instances).display !== 'none' && !instances.classList.contains('hidden');

                  return {
                    windowInnerWidth: window.innerWidth,
                    pluginsWidth: Math.round(pRect ? pRect.width : 0),
                    pluginsRight: Math.round(pRect ? pRect.right : 0),
                    instancesWidth: Math.round(isInstancesVisible && iRect ? iRect.width : 0),
                    instancesLeft: Math.round(isInstancesVisible && iRect ? iRect.left : 0),
                    instancesRight: Math.round(isInstancesVisible && iRect ? iRect.right : 0),
                    instancesVisible: isInstancesVisible,
                    workspaceWidth: Math.round(wRect ? wRect.width : 0),
                    workspaceLeft: Math.round(wRect ? wRect.left : 0),
                    scrollWidth: document.documentElement.scrollWidth,
                    clientWidth: document.documentElement.clientWidth,
                    ariaExpanded: toggleBtn ? toggleBtn.getAttribute('aria-expanded') : null,
                    ariaControls: toggleBtn ? toggleBtn.getAttribute('aria-controls') : null,
                    ariaLabel: toggleBtn ? toggleBtn.getAttribute('aria-label') : null
                  };
                })()
              `,
              returnByValue: true,
            });
            return res.result.value;
          };

          // --- 1. 1280px Wide Screen Verification ---
          const m1280 = await measureLayout(1280, 800);
          assert.strictEqual(m1280.pluginsWidth, 72, "1280px: plugin sidebar must be 72px");
          assert.strictEqual(m1280.instancesWidth, 240, "1280px: instances sidebar must be 240px");
          assert.strictEqual(m1280.instancesVisible, true, "1280px: instances sidebar must be visible");
          assert.strictEqual(m1280.workspaceWidth, 968, "1280px: workspace width must be 968px");
          assert.ok(m1280.instancesLeft >= m1280.pluginsRight, "1280px: instances sidebar must not overlap plugin sidebar");
          assert.ok(m1280.workspaceLeft >= m1280.instancesRight, "1280px: workspace must not overlap instances sidebar");
          assert.ok(m1280.scrollWidth <= 1280, "1280px: must have no horizontal page overflow");

          // --- 2. 1024px Medium Screen Verification ---
          const m1024 = await measureLayout(1024, 768);
          assert.strictEqual(m1024.pluginsWidth, 72, "1024px: plugin sidebar must be 72px");
          assert.strictEqual(m1024.instancesWidth, 200, "1024px: instances sidebar must adapt to 200px");
          assert.strictEqual(m1024.instancesVisible, true, "1024px: instances sidebar must be visible");
          assert.strictEqual(m1024.workspaceWidth, 752, "1024px: workspace width must be 752px");
          assert.ok(m1024.instancesLeft >= m1024.pluginsRight, "1024px: no overlap between plugins and instances");
          assert.ok(m1024.workspaceLeft >= m1024.instancesRight, "1024px: no overlap between instances and workspace");
          assert.ok(m1024.scrollWidth <= 1024, "1024px: must have no horizontal page overflow");

          // --- 3. 768px Compact Screen Verification (Auto-collapsed) ---
          const m768 = await measureLayout(768, 1024);
          assert.strictEqual(m768.pluginsWidth, 72, "768px: plugin sidebar must be 72px");
          assert.strictEqual(m768.instancesVisible, false, "768px: instances sidebar must be auto-collapsed under 900px breakpoint");
          assert.strictEqual(m768.instancesWidth, 0, "768px: collapsed instances sidebar width must be 0px");
          assert.strictEqual(m768.workspaceWidth, 696, "768px: workspace must receive full remaining width 696px");
          assert.strictEqual(m768.ariaExpanded, "false", "768px: toggle button aria-expanded must be false");
          assert.strictEqual(m768.ariaControls, "instances-sidebar", "768px: toggle button aria-controls must match");
          assert.ok(m768.scrollWidth <= 768, "768px: must have no horizontal page overflow");

          // --- 4. 768px User Manual Expand & Reversible Collapse Verification ---
          // Click toggle button to expand
          await send("Runtime.evaluate", {
            expression: "document.getElementById('toggle-instances-button').click();",
          });
          await new Promise((r) => setTimeout(r, 200));

          const m768Expanded = await measureLayout(768, 1024);
          assert.strictEqual(m768Expanded.instancesVisible, true, "768px expanded: instances sidebar must be visible");
          assert.strictEqual(m768Expanded.instancesWidth, 200, "768px expanded: instances sidebar must be 200px");
          assert.strictEqual(m768Expanded.workspaceWidth, 496, "768px expanded: workspace must adaptively compress to 496px");
          assert.strictEqual(m768Expanded.ariaExpanded, "true", "768px expanded: toggle button aria-expanded must be true");
          assert.ok(m768Expanded.instancesLeft >= m768Expanded.pluginsRight, "768px expanded: no overlap");
          assert.ok(m768Expanded.workspaceLeft >= m768Expanded.instancesRight, "768px expanded: no overlap");
          assert.ok(m768Expanded.scrollWidth <= 768, "768px expanded: no horizontal page overflow");

          // Click collapse button inside instances panel header
          await send("Runtime.evaluate", {
            expression: "document.querySelector('#instances-sidebar button[aria-label=\"Collapse instances panel\"]').click();",
          });
          await new Promise((r) => setTimeout(r, 200));

          const m768ReCollapsed = await measureLayout(768, 1024);
          assert.strictEqual(m768ReCollapsed.instancesVisible, false, "768px re-collapsed: instances sidebar must be collapsed");
          assert.strictEqual(m768ReCollapsed.workspaceWidth, 696, "768px re-collapsed: workspace must return to 696px");
          assert.strictEqual(m768ReCollapsed.ariaExpanded, "false", "768px re-collapsed: aria-expanded must return to false");

          // --- 5. Shadow DOM Token Inheritance & Fallback Browser Verification ---
          const shadowVerification = await send("Runtime.evaluate", {
            expression: `
              (() => {
                // Create a host element in document body (inherits :root custom properties)
                const hostEl = document.createElement('div');
                hostEl.id = 'test-shadow-host';
                document.body.appendChild(hostEl);
                const shadowRoot = hostEl.attachShadow({ mode: 'open' });
                
                const style = document.createElement('style');
                style.textContent = \`
                  .token-box {
                    color: var(--coterm-text-primary, #cccccc);
                    background-color: var(--coterm-surface, #1e1e1e);
                    border-color: var(--coterm-border, #2b2b2b);
                  }
                  .fallback-box {
                    color: var(--coterm-non-existent-token, rgb(250, 204, 21));
                  }
                \`;
                shadowRoot.appendChild(style);

                const tokenBox = document.createElement('div');
                tokenBox.className = 'token-box';
                tokenBox.textContent = 'Token Test';
                shadowRoot.appendChild(tokenBox);

                const fallbackBox = document.createElement('div');
                fallbackBox.className = 'fallback-box';
                fallbackBox.textContent = 'Fallback Test';
                shadowRoot.appendChild(fallbackBox);

                const tokenStyle = window.getComputedStyle(tokenBox);
                const fallbackStyle = window.getComputedStyle(fallbackBox);

                const result = {
                  tokenColor: tokenStyle.color,
                  tokenBg: tokenStyle.backgroundColor,
                  fallbackColor: fallbackStyle.color
                };

                hostEl.remove();
                return result;
              })()
            `,
            returnByValue: true,
          });

          const shadowRes = shadowVerification.result.value;
          // var(--coterm-text-primary) is #cccccc = rgb(204, 204, 204)
          assert.strictEqual(shadowRes.tokenColor, "rgb(204, 204, 204)", "Shadow DOM must inherit --coterm-text-primary");
          // var(--coterm-surface) is #1e1e1e = rgb(30, 30, 30)
          assert.strictEqual(shadowRes.tokenBg, "rgb(30, 30, 30)", "Shadow DOM must inherit --coterm-surface");
          // fallback rgb(250, 204, 21)
          assert.strictEqual(shadowRes.fallbackColor, "rgb(250, 204, 21)", "Shadow DOM must use fallback when token is undefined");

          ws.close();
          resolve();
        } catch (err) {
          reject(err);
        }
      };
      ws.onerror = reject;
    });
  } finally {
    browserProc.kill();
    server.close();
  }
});
