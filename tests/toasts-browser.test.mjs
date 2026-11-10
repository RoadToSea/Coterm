import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("shared toasts are non-modal, colored, bounded, dismissible and automatically released", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; shared notifications were not verified"); return; }
  const artifact = new URL("../apps/shared/dist/browser/toasts.js", import.meta.url);
  assert.ok(fs.statSync(artifact).mtimeMs >= fs.statSync(new URL("../apps/shared/components/toasts.ts", import.meta.url)).mtimeMs,
    "run pnpm test:build before testing the shared toast component");
  const url = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({ browserPath, totalTimeoutMs: 20_000 });
  const run = source => evaluate(session.client, source);
  try {
    await session.client.send("Runtime.enable");
    await session.client.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await run(`(async () => {
      document.body.style.cssText='margin:0;height:600px';
      const host=document.createElement('div'); document.body.append(host); window.root=host.attachShadow({mode:'open'});
      window.area=document.createElement('div'); area.style.cssText='position:relative;height:500px;--coterm-danger:rgb(180,30,40);--coterm-warning:rgb(190,140,20);--coterm-surface-raised:#fff;--coterm-text-primary:#222'; root.append(area);
      window.input=document.createElement('textarea'); area.append(input); input.focus();
      const {mountToasts}=await import(${JSON.stringify(url)});
      window.toasts=mountToasts(area,{durationMs:500,maxVisible:3});
      window.closeError=toasts.show({message:'Connection failed',tone:'error'});
      toasts.show({message:'Invalid HEX input',tone:'warning'});
    })()`);
    assert.equal(await run(`root.activeElement===input`), true, "showing a toast keeps the editor focused");
    assert.equal(await run(`root.querySelector('dialog,[aria-modal=true]')===null`), true);
    assert.equal(await run(`getComputedStyle(root.querySelector('.coterm-toasts')).pointerEvents`), "none");
    assert.equal(await run(`getComputedStyle(root.querySelector('[data-tone=error]')).borderTopColor`), "rgb(180, 30, 40)");
    assert.equal(await run(`getComputedStyle(root.querySelector('[data-tone=warning]')).borderTopColor`), "rgb(190, 140, 20)");
    assert.equal(await run(`root.querySelector('[data-tone=warning]').getAttribute('role')`), "alert");
    assert.ok(Number.parseFloat(await run(`getComputedStyle(root.querySelector('.coterm-toast')).borderRadius`)) > 0);
    await waitForValue(session.client, `root.querySelectorAll('.coterm-toast').length===0`);
    assert.equal(await run(`root.activeElement===input`), true, "expiry never changes focus");

    await run(`for(let i=0;i<10;i++) toasts.show({message:'same warning',tone:'warning'})`);
    assert.equal(await run(`root.querySelectorAll('.coterm-toast').length`), 1, "identical messages are coalesced");
    await run(`for(let i=0;i<100;i++) toasts.show({message:'error '+i,tone:'error'})`);
    assert.equal(await run(`root.querySelectorAll('.coterm-toast').length`), 3);
    await run(`toasts.clear(); toasts.show({message:'<img src=x onerror=alert(1)>',tone:'warning'});
      root.querySelector('.coterm-toast').dispatchEvent(new MouseEvent('mouseenter'))`);
    assert.equal(await run(`root.querySelector('.coterm-toast img')===null`), true, "messages stay plain text");
    await run(`new Promise(resolve=>setTimeout(resolve,650))`);
    assert.equal(await run(`root.querySelectorAll('.coterm-toast').length`), 1, "hover pauses expiry for reading");
    await run(`root.querySelector('.coterm-toast').dispatchEvent(new MouseEvent('mouseleave'))`);
    await waitForValue(session.client, `root.querySelectorAll('.coterm-toast').length===0`);

    await run(`toasts.show({message:'keyboard reading',tone:'error'}); root.querySelector('.coterm-toast-close').focus()`);
    assert.equal(await run(`root.activeElement===root.querySelector('.coterm-toast-close')`), true, "the toast close button is keyboard focusable");
    await run(`new Promise(resolve=>setTimeout(resolve,650))`);
    assert.equal(await run(`root.querySelectorAll('.coterm-toast').length`), 1, "keyboard focus pauses expiry");
    await run(`input.focus()`);
    await waitForValue(session.client, `root.querySelectorAll('.coterm-toast').length===0`);
    await run(`toasts.show({message:'close me',tone:'warning'}); toasts.updateLabels({ariaLabel:'提示',closeLabel:'关闭提示'})`);
    assert.equal(await run(`root.querySelector('.coterm-toasts').getAttribute('aria-label')`), "提示");
    assert.equal(await run(`root.querySelector('.coterm-toast-close').getAttribute('aria-label')`), "关闭提示");
    await run(`root.querySelector('.coterm-toast-close').click()`);
    assert.equal(await run(`root.querySelectorAll('.coterm-toast').length`), 0);
    await run(`toasts.show({message:'pending timer'}); toasts.unmount(); closeError(); toasts.show({message:'after unmount'})`);
    await run(`new Promise(resolve=>setTimeout(resolve,650))`);
    assert.equal(await run(`area.children.length`), 1, "unmount clears timers, notices and component styles");
  } finally { await session.close(); }
});
