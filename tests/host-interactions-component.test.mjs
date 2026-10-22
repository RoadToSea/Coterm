/**
 * 真实组件的交互生命周期验收：在浏览器里挂载 HostInteractions，让真实订阅、真实 secret-input
 * 与真实 IPC 契约一起跑，验证弹窗队列的身份归属。
 *
 * 与 `host-attention-state.test.mjs` 的分工：
 * - 那边测 host-api 的订阅复用与代次（手写 IPC 支架，无 DOM）；
 * - 这边测组件与 IPC 的协作（真实构建产物 + 可控 Promise），旧 finally 不能干扰新交互。
 *
 * 没有浏览器时整组 skip；skip 不等于通过。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { delay, evaluate, findBrowser, openSession, startStaticServer, waitForValue } from "./support/browser-harness.mjs";
import { assertFreshTestArtifacts } from "./support/browser-harness.mjs";
import { makeHostMockSource, secretInteraction } from "./support/host-mock.js";

const browserPath = findBrowser();

const attention = (version, interactions) => ({ version, approvals: [], interactions });

/** 打开只挂 HostInteractions 的页面；宿主替身先于页面脚本注入。 */
async function openInteractionsPage(session, scenario) {
  const { client } = session;
  const target = await client.send("Target.createTarget", { url: "about:blank" });
  const attached = await client.send("Target.attachToTarget", { targetId: target.targetId, flatten: true });
  const sessionId = attached.sessionId;
  const send = (method, params = {}) => client.send(method, params, sessionId);
  const waitFor = (method, timeoutMs) => client.waitFor(method, timeoutMs, sessionId);

  await send("Page.enable");
  await send("Runtime.enable");
  client.on("Runtime.exceptionThrown", (params) => {
    console.error("[BROWSER EXCEPTION]", params.exceptionDetails.text, params.exceptionDetails.exception?.description);
  }, sessionId);
  await send("Page.addScriptToEvaluateOnNewDocument", { source: makeHostMockSource(scenario) });

  const page = {
    send,
    waitFor,
    evaluate: (expression) => evaluate({ send }, expression),
    async close() {
      await client.send("Target.closeTarget", { targetId: target.targetId }).catch(() => {});
    },
  };

  const loaded = waitFor("Page.loadEventFired", 20_000);
  await send("Page.navigate", { url: `${serverUrl}__host-interactions.html` });
  await loaded;
  // 页面脚本只注册入口对象；挂载由用例显式触发，便于在挂载前后摆放 IPC 状态。
  await waitForValue({ send }, `typeof window.__cotermHostInteractions?.mount === "function"`, { label: "entry loaded" });
  await page.evaluate("window.__cotermHostInteractions.mount()");
  return page;
}

let serverUrl;

function scenarioBase(initialAttention) {
  return {
    catalog: { plugins: [], issues: [] },
    instances: [],
    bypass: false,
    attention: initialAttention,
    failures: {},
    sshFrontendSource: null,
  };
}

/** 等待弹窗数量稳定为 n（DOM 层是唯一的真相来源）。 */
async function waitForDialogs(page, count, label) {
  await waitForValue(
    { send: page.send },
    `document.querySelectorAll("dialog.host-secret-dialog").length === ${count}`,
    { label },
  );
}

/**
 * 取一次页面快照。反复重开同名弹窗会让渲染进程卡住、CDP 求值超时；
 * 那本身就是缺陷证据，所以把它变成一次可读的断言而不是让用例挂在超时里。
 */
async function sample(page, expression) {
  try {
    return { ok: true, value: await page.evaluate(expression) };
  } catch (error) {
    if (/Timed out after \d+ms: Runtime\.evaluate/.test(String(error))) return { ok: false };
    throw error;
  }
}

test("HostInteractions: a departed request's late finally cannot reopen it or clear the new one", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; real-component interaction verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    // A 先到，B 随后在同一快照里取代 A（宿主已移除 A）。
    const page = await openInteractionsPage(session, scenarioBase(attention(1, [secretInteraction("int-a", "inst-a")])));
    try {
      await waitForDialogs(page, 1, "A opened");
      const first = await page.evaluate(`window.__cotermTest.dialogs()[0].title`);
      assert.match(first, /inst-a/, "first dialog belongs to A");

      // 宿主收敛 A、同时送来 B。
      // 旧实现会在 A 的 finally 里无条件 active=null 再补取，重开 B；
      // promptForSecret 又按同一个 key 取消同名弹窗，于是反复开关把渲染进程拖死。
      // 所以收敛这一步本身就要求页面还能应答；不应答就已经是缺陷证据。
      const switched = await sample(page, `(() => {
        window.__cotermTest.setAttention(${JSON.stringify(attention(2, [secretInteraction("int-b", "inst-b")]))});
        return true;
      })()`);
      assert.ok(
        switched.ok,
        "page must stay responsive across A departing and B arriving; a reopen storm wedges the renderer",
      );
      await delay(500);
      const afterSwitch = await sample(page, `(() => ({
        dialogs: window.__cotermTest.dialogs().map((d) => d.title),
        hostAttentionCalls: window.__cotermTest.countCalls("host_attention"),
      }))()`);
      assert.ok(afterSwitch.ok, "page still responsive after the stale finally would have fired");
      assert.equal(
        afterSwitch.value.dialogs.length,
        1,
        `exactly one dialog after A departed (saw ${afterSwitch.value.dialogs.length})`,
      );
      assert.match(afterSwitch.value.dialogs[0], /inst-b/, "remaining dialog belongs to B, not a reopened A");

      // 再推一格，确认旧 finally 不会额外补取、也不会重开。
      await delay(300);
      const settled = await sample(page, `(() => ({
        dialogs: window.__cotermTest.dialogs().map((d) => d.title),
        hostAttentionCalls: window.__cotermTest.countCalls("host_attention"),
      }))()`);
      assert.ok(settled.ok, "page still responsive after the stale finally would have fired");
      assert.equal(settled.value.dialogs.length, 1, "no duplicate dialog from the stale finally");
      assert.match(settled.value.dialogs[0], /inst-b/, "the surviving dialog is still B");
      assert.equal(
        settled.value.hostAttentionCalls,
        afterSwitch.value.hostAttentionCalls,
        "a departed prompt must not trigger an extra hostAttention reload",
      );

      // B 提交后队列继续收敛，不残留、不串实例。
      await page.evaluate(`window.__cotermTest.submitSecret("pw-for-b")`);
      await waitForValue({ send: page.send }, `window.__cotermTest.pendingSecrets.length === 1`, { label: "B submitted through respond_secret" });
      const submitted = await page.evaluate(`window.__cotermTest.pendingSecrets[0]`);
      assert.equal(submitted.interactionId, "int-b", "respond_secret carries B's interaction id");
      assert.equal(submitted.value, "pw-for-b", "respond_secret carries the typed value");
      await waitForDialogs(page, 0, "queue drained to zero dialogs");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
    await new Promise((resolve) => server.close(resolve));
  }
});

test("HostInteractions: late IPC completion from an old prompt does not clear or reopen the new one", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; late-IPC verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    const page = await openInteractionsPage(session, scenarioBase(attention(1, [secretInteraction("int-1", "inst-1")])));
    try {
      await waitForDialogs(page, 1, "first prompt open");

      // 挂起 respond_secret，让第一次提交停在「deliver 已发出、结果未回」的窗口。
      await page.evaluate(`window.__respondGate = window.__cotermTest.gate("respond_secret")`);
      await page.evaluate(`window.__cotermTest.submitSecret("pw-one")`);
      await waitForValue({ send: page.send }, `window.__respondGate.size() === 1`, { label: "respond_secret in flight" });
      // 提交会先清空输入并关闭对话框吗？不会：finish 只在 deliver resolve 之后调用。
      await waitForDialogs(page, 1, "dialog stays open while delivery is in flight");

      // 宿主此时收敛 1 并给出 2：旧弹窗必须被关闭，新弹窗必须打开。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, [secretInteraction("int-2", "inst-2")]))})`);
      await waitForDialogs(page, 1, "old prompt closed on host convergence");
      const titles = await page.evaluate(`window.__cotermTest.dialogs().map((d) => d.title)`);
      assert.match(titles[0], /inst-2/, "prompt 2 owns the dialog after host convergence");

      // 放行第一次投递：迟到完成只处理自己，不得清空新 active 或重开旧项。
      await page.evaluate(`window.__respondGate.release(null)`);
      await delay(200);
      const afterLate = await page.evaluate(`(() => ({
        titles: window.__cotermTest.dialogs().map((d) => d.title),
        deliveries: window.__cotermTest.invocations
          .filter((item) => item.command === "respond_secret")
          .map((item) => [item.args.interactionId, item.args.value]),
      }))()`);
      assert.equal(afterLate.titles.length, 1, "no extra dialog after the late delivery resolved");
      assert.match(afterLate.titles[0], /inst-2/, "prompt 2 still owns the single dialog");
      assert.deepEqual(afterLate.deliveries, [["int-1", "pw-one"]], "only prompt 1's own delivery was sent");

      // prompt 2 提交后，秘密必须属于 int-2，不能串到 int-1。
      await page.evaluate(`window.__cotermTest.clearGates()`);
      await page.evaluate(`window.__cotermTest.submitSecret("pw-two")`);
      await waitForValue({ send: page.send }, `window.__cotermTest.pendingSecrets.length === 1`, { label: "prompt 2 submitted" });
      const ids = await page.evaluate(`(() => ({
        handled: window.__cotermTest.pendingSecrets.map((item) => [item.interactionId, item.value]),
        invoked: window.__cotermTest.invocations
          .filter((item) => item.command === "respond_secret")
          .map((item) => [item.args.interactionId, item.args.value]),
      }))()`);
      assert.deepEqual(ids.invoked, [["int-1", "pw-one"], ["int-2", "pw-two"]], "each value stays bound to its own interaction");
      assert.deepEqual(ids.handled, [["int-2", "pw-two"]], "only the ungated delivery reached the host handler");
      await waitForDialogs(page, 0, "queue drained");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
    await new Promise((resolve) => server.close(resolve));
  }
});

test("HostInteractions: cancel, expiry, instance destroy, plugin stop and unmount all converge", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; lifecycle convergence verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 180_000 });

  try {
    const page = await openInteractionsPage(session, scenarioBase(attention(1, [secretInteraction("int-cancel", "inst-1")])));
    try {
      // 1. 用户点取消：弹窗关闭且队列可以继续。
      await waitForDialogs(page, 1, "cancel case open");
      await page.evaluate(`window.__cotermTest.clickCancel()`);
      await waitForDialogs(page, 0, "user cancel closes the dialog");
      await waitForValue({ send: page.send }, `window.__cotermTest.pendingSecrets.length === 1`, { label: "cancel notifies the host with a null value" });
      const cancelCall = await page.evaluate(`window.__cotermTest.pendingSecrets[0]`);
      assert.deepEqual(cancelCall, { interactionId: "int-cancel", value: null }, "cancel sends respond_secret(int-cancel, null)");

      // 2. 取消后迟到的 submit 不得提交任何秘密。
      const before = await page.evaluate(`window.__cotermTest.pendingSecrets.length`);
      const staleSubmit = await page.evaluate(`(() => {
        try { window.__cotermTest.submitSecret("should-not-send"); return "submitted"; }
        catch (error) { return String(error); }
      })()`);
      assert.match(staleSubmit, /no secret dialog open/, "closed dialog cannot accept a late submit");
      assert.equal(await page.evaluate(`window.__cotermTest.pendingSecrets.length`), before, "no secret delivered after cancel");

      // 3. 宿主在到期前直接收敛（过期）：弹窗关闭，队列继续。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, [secretInteraction("int-expire", "inst-1")]))})`);
      await waitForDialogs(page, 1, "expiry case open");
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(3, []))})`);
      await waitForDialogs(page, 0, "host-side expiry closes the dialog without a secret");

      // 4. 实例销毁（宿主删掉该实例的交互，同时保留另一实例的请求）。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(4, [secretInteraction("int-destroy", "inst-doomed")]))})`);
      await waitForDialogs(page, 1, "doomed instance prompt open");
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(5, [secretInteraction("int-survivor", "inst-alive")]))})`);
      await waitForDialogs(page, 1, "destroyed instance prompt replaced by the surviving one");
      assert.match(await page.evaluate(`window.__cotermTest.dialogs()[0].title`), /inst-alive/, "only the surviving instance prompts");

      // 5. 插件停止：宿主清空全部交互。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(6, []))})`);
      await waitForDialogs(page, 0, "plugin stop removes every prompt");

      // 6. 卸载：disposed 之后即使有新快照，也不得弹窗。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(7, [secretInteraction("int-unmount", "inst-1")]))})`);
      await waitForDialogs(page, 1, "prompt open before unmount");
      await page.evaluate("window.__cotermHostInteractions.unmount()");
      await waitForDialogs(page, 0, "unmount closes the open dialog");
      const listenersAfterUnmount = await page.evaluate(`window.__cotermTest.listenerCount()`);
      assert.equal(listenersAfterUnmount, 0, "unmount removes the attention listener");
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(8, [secretInteraction("int-late", "inst-1")]))})`);
      await delay(200);
      await waitForDialogs(page, 0, "no dialog after unmount");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
    await new Promise((resolve) => server.close(resolve));
  }
});

test("HostInteractions: duplicate and out-of-order versions do not double-prompt", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; duplicate/out-of-order verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    const request = secretInteraction("int-dup", "inst-dup");
    const page = await openInteractionsPage(session, scenarioBase(attention(10, [request])));
    try {
      await waitForDialogs(page, 1, "initial prompt open");

      // 同一版本重复通知、以及比当前更旧的版本，都不能再开第二个弹窗。
      await page.evaluate(`window.__cotermTest.emitAttention(10)`);
      await page.evaluate(`window.__cotermTest.emitAttention(9)`);
      await delay(200);
      await waitForDialogs(page, 1, "duplicate and stale versions keep a single dialog");

      // 版本推进但内容不变：仍然只开一个。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(11, [request]))})`);
      await delay(200);
      await waitForDialogs(page, 1, "unchanged content at a newer version does not reopen");

      // 提交一次即收敛；迟到的重复版本不能再把它翻出来。
      await page.evaluate(`window.__cotermTest.submitSecret("pw-dup")`);
      await waitForValue({ send: page.send }, `window.__cotermTest.pendingSecrets.length === 1`, { label: "duplicate case submitted once" });
      await page.evaluate(`window.__cotermTest.emitAttention(11)`);
      await delay(200);
      await waitForDialogs(page, 0, "a resolved request is not reopened by a repeated version");
      assert.equal(await page.evaluate(`window.__cotermTest.pendingSecrets.length`), 1, "exactly one delivery for one interaction");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
    await new Promise((resolve) => server.close(resolve));
  }
});
