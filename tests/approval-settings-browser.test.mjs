import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { assertFreshBuild, evaluate, findBrowser, openSession, setAppLocale, startStaticServer, waitForValue } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("command approval settings edit both modes, preserve failed saves and preview through the host", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshBuild();
  const scenario = structuredClone(defaultScenario);
  scenario.approvalPreview = { "ls": false, "rm x": true };
  const server = await startStaticServer();
  const session = await openSession({ browserPath });
  const { client } = session;
  const run = source => evaluate(client, source);
  const wait = expression => waitForValue(client, expression, { label: "command approval settings" });
  const input = async (id, value) => {
    await run(`(() => { const input = document.getElementById(${JSON.stringify(id)});
      Object.getOwnPropertyDescriptor(input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, 'value').set.call(input, ${JSON.stringify(value)});
      input.dispatchEvent(new Event('input', {bubbles:true})); })()`);
  };
  try {
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Page.addScriptToEvaluateOnNewDocument", { source: makeHostMockSource(scenario) });
    await client.send("Page.navigate", { url: server.url });
    await wait(`!!document.querySelector('button[aria-label="Open approval settings"]')`);
    await run(`document.querySelector('button[aria-label="Open approval settings"]').click()`);
    await wait(`!!document.getElementById('approval-pattern')`);
    assert.equal(await run(`__cotermTest.countCalls('list_operations')`), 0);
    await input("approval-pattern", "^ls$");
    await run(`[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Add rule').click()`);
    await input("approval-pattern", "^cd /tmp$");
    await run(`[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Add rule').click()`);
    await run(`window.saveGate=__cotermTest.gate('set_approval_policy');[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Save rules').click()`);
    await wait(`saveGate.size()===1`);
    assert.equal(await run(`document.querySelector('#approval-settings-dialog fieldset').disabled`), true);
    await run(`saveGate.fail('disk unavailable')`);
    await wait(`document.querySelector('#approval-settings-dialog [role=alert]')?.textContent.includes('disk unavailable')`);
    assert.deepEqual(await run(`[...document.querySelectorAll('#approval-settings-dialog code')].map(el=>el.textContent)`), ["^ls$", "^cd /tmp$"]);
    assert.deepEqual(await run(`__cotermTest.workspace().approvalPolicy.patterns`), []);
    await run(`[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Save rules').click()`);
    await wait(`__cotermTest.workspace().approvalPolicy.patterns.length===2`);
    await input("approval-preview", "ls");
    await run(`[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Test matching').click()`);
    await wait(`document.getElementById('approval-settings-dialog').textContent.includes('Skips human approval')`);
    await run(`[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Intercept matching commands').click()`);
    await run(`[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Save rules').click()`);
    await wait(`__cotermTest.workspace().approvalPolicy.mode==='approve_matches'`);
    await run(`document.querySelector('#approval-settings-dialog button[aria-label="Remove rule 1"]').click()`);
    await run(`[...document.querySelectorAll('#approval-settings-dialog button')].find(button=>button.textContent==='Save rules').click()`);
    await wait(`__cotermTest.workspace().approvalPolicy.patterns.length===1`);
    await run(`document.querySelector('#approval-settings-dialog button[aria-label="Close dialog"]').click()`);
    await setAppLocale(client, "zh-CN");
    await run(`document.querySelector('button[aria-label="打开审批设置"]').click()`);
    await wait(`document.getElementById('approval-settings-dialog')?.textContent.includes('反向拦截')`);
    assert.equal(await run(`document.querySelector('#approval-settings-dialog code').textContent`), "^cd /tmp$");
    assert.equal(await run(`__cotermTest.invocations.find(call=>call.command==='preview_approval_policy').args.text`), "ls");
    await client.send("Emulation.setDeviceMetricsOverride", { width: 420, height: 800, deviceScaleFactor: 1, mobile: false });
    assert.equal(await run(`document.getElementById('approval-settings-dialog').scrollWidth <= document.getElementById('approval-settings-dialog').clientWidth`), true);
    fs.mkdirSync(new URL("../target/", import.meta.url), { recursive: true });
    const screenshot = await client.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(new URL("../target/approval-settings-narrow.png", import.meta.url), Buffer.from(screenshot.data, "base64"));
  } finally {
    await session.close();
    await new Promise(resolve => server.server.close(resolve));
  }
});
