/** Actual desktop -> PluginAPI -> Serial frontend; both saved instances use the same COM port. */
import test from "node:test";
import assert from "node:assert/strict";
import { assertFreshBuild, evaluate, findBrowser, openSession, startStaticServer, waitForValue } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("desktop forwards saved instance identity across restart, name changes and shared COM ports", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary"); return; }
  assertFreshBuild();
  const scenario = structuredClone(defaultScenario);
  scenario.catalog = { plugins: [{ id: "serial", name: "Serial", version: "0.1.0", hasFrontend: true, running: true }], issues: [] };
  scenario.instances = [
    { pluginId: "serial", pluginInstanceId: "runtime-one", sessionId: "session-one", label: "Board A", status: "ready" },
    { pluginId: "serial", pluginInstanceId: "runtime-two", sessionId: "session-two", label: "Board B", status: "ready" },
  ];
  scenario.workspace = { entries: scenario.instances.map((instance, index) => ({
    pluginId: "serial", configRef: "saved-" + index, instanceId: instance.pluginInstanceId, label: instance.label, workspaceId: "default",
  })) };
  const server = await startStaticServer();
  const session = await openSession({ browserPath });
  const { client } = session;
  const run = source => evaluate(client, source);
  const wait = source => waitForValue(client, source);
  const open = async id => {
    await wait(`!!document.querySelector('[data-instance-id="${id}"] > button')`);
    await run(`document.querySelector('[data-instance-id="${id}"] > button').click();
      window.serialRoot=()=>Array.from(document.querySelector('[role=tabpanel]:not([hidden])').querySelectorAll('div'))
        .map(el=>el.shadowRoot).find(root=>root?.querySelector('.connect-button'));`);
    await wait(`window.serialRoot()?.querySelector('.connect-button').dataset.connected==='true'`);
  };
  try {
    await client.send("Page.enable"); await client.send("Runtime.enable");
    let script = await client.send("Page.addScriptToEvaluateOnNewDocument", {
      source: makeHostMockSource(scenario) + `\nwindow.workspaceGate=__cotermTest.gate('workspace_snapshot');`,
    });
    await client.send("Page.navigate", { url: server.url });
    await wait(`!!document.querySelector('[data-instance-id="runtime-one"] > button') && workspaceGate.size()===1`);
    await run(`document.querySelector('[data-instance-id="runtime-one"] > button').click()`);
    assert.equal(await run(`Array.from(document.querySelectorAll('main div')).some(el=>el.shadowRoot?.querySelector('.xterm'))`), false,
      "wait for saved identity before mounting; a late workspace response must not discard an edited frontend");
    await run(`workspaceGate.release(__cotermTest.workspace())`);
    await open("runtime-one");
    await run(`serialRoot().querySelector('.settings-toggle').click();serialRoot().querySelector('[name=receiveCompleteLF]').click()`);
    assert.equal(await run(`JSON.parse(localStorage.getItem('coterm.serial.display.v1:config:saved-0')).receiveNewlines`), "lf");
    await open("runtime-two");
    assert.equal(await run(`serialRoot().querySelector('[name=receiveCompleteLF]').checked`), false,
      "a different saved instance on the same COM port must keep its own display choices");

    scenario.instances[0] = { ...scenario.instances[0], pluginInstanceId: "recreated-one", sessionId: "recreated-session", label: "Renamed board" };
    scenario.workspace.entries[0] = { ...scenario.workspace.entries[0], instanceId: "recreated-one", label: "Renamed board" };
    await client.send("Page.removeScriptToEvaluateOnNewDocument", { identifier: script.identifier });
    script = await client.send("Page.addScriptToEvaluateOnNewDocument", { source: makeHostMockSource(scenario) });
    await client.send("Page.reload");
    await open("recreated-one");
    assert.equal(await run(`serialRoot().querySelector('[name=receiveCompleteLF]').checked`), true);
    assert.equal(await run(`__cotermTest.invocations.some(call=>['serial.write','serial.write_hex'].includes(call.args.operation))`), false);
  } finally { await session.close(); await new Promise(resolve => server.server.close(resolve)); }
});
