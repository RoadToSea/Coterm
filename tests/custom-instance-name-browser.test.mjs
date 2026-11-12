import test from "node:test";
import assert from "node:assert/strict";
import { assertFreshBuild, evaluate, findBrowser, openSession, startStaticServer, waitForValue } from "./support/browser-harness.mjs";
import { defaultScenario, makeHostMockSource } from "./support/host-mock.js";

test("the shared driver configuration submits a separate custom instance name", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; custom instance naming was not browser-verified"); return; }
  assertFreshBuild();
  const scenario = structuredClone(defaultScenario);
  scenario.catalog.issues = [];
  const server = await startStaticServer();
  const session = await openSession({ browserPath });
  const { client } = session;
  const run = source => evaluate(client, source);
  const wait = expression => waitForValue(client, expression, { label: "custom instance name" });
  try {
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Page.addScriptToEvaluateOnNewDocument", { source: makeHostMockSource(scenario) });
    await client.send("Page.navigate", { url: server.url });
    await wait(`!!document.querySelector('button[aria-label="Manage SSH"]')`);
    await run(`document.querySelector('button[aria-label="Manage SSH"]').click()`);
    await wait(`!!document.getElementById('new-instance-name')`);
    await wait(`Array.from(document.querySelectorAll('#driver-configuration div')).some(element => element.shadowRoot?.querySelector('form'))`);
    await run(`(() => {
      window.configuration = [...document.querySelectorAll('#driver-configuration div')]
        .find(element => element.shadowRoot?.querySelector('form'))?.shadowRoot;
      const input = document.getElementById('new-instance-name');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, 'Lab server');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      const fields = configuration.querySelectorAll('input');
      fields[0].value = 'lab.example';
      fields[2].value = 'tester';
      configuration.querySelector('form').requestSubmit();
    })()`);
    await wait(`__cotermTest.countCalls('create_instance') === 1`);
    const args = await run(`__cotermTest.invocations.find(call => call.command === 'create_instance').args`);
    assert.equal(args.name, "Lab server");
    assert.deepEqual(args.config, { host: "lab.example", port: 22, username: "tester" });
    assert.equal(await run(`__cotermTest.workspace().entries.find(entry => entry.instanceId === 'created-1').name`), "Lab server");
  } finally {
    await session.close();
    await new Promise(resolve => server.server.close(resolve));
  }
});
