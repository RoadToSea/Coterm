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

/**
 * 让一个已放行的闸门真正走完：先排空 Promise 微任务，再等两跳宏任务。
 * 微任务链在同一个任务里跑完（弹窗是同步插入 DOM 的），
 * 宏任务跳转则是给 React 的 Scheduler 落地外部 setState 用。不用固定 sleep 赌先后。
 */
async function flushPage(page) {
  await page.evaluate(`(async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
    for (let i = 0; i < 2; i++) {
      await new Promise((resolve) => {
        const channel = new MessageChannel();
        channel.port1.onmessage = () => resolve(true);
        channel.port2.postMessage(null);
      });
    }
    return true;
  })()`);
}

/** 弹窗标题，用于确认归结到零以后没有旧请求被翻出来。 */
async function dialogTitles(page) {
  return page.evaluate(`window.__cotermTest.dialogs().map((d) => d.title)`);
}

test("HostInteractions: a departed request's late finally cannot reopen it or clear the new one", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; real-component interaction verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
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
  }
});

test("HostInteractions: late IPC completion from an old prompt does not clear or reopen the new one", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; late-IPC verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
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
  }
});

test("HostInteractions: cancel, expiry, instance destroy, plugin stop and unmount all converge", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; lifecycle convergence verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
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
  }
});

test("HostInteractions: duplicate and out-of-order versions do not double-prompt", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; duplicate/out-of-order verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
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
  }
});

/**
 * 下面的补取用例回归同一个缺陷：finally 里的补取 `hostAttention()` 只看 `disposed` / `active`，
 * 不看自己相对共享订阅的先后。较新的空快照先把 A 从 settled 里清掉，早先补取随后带着
 * 旧版本 `[A]` 回来时 active 恰好为空，A 就被重新打开了。
 *
 * 顺序全部由可控闸门决定。闸门按调用顺序排队，所以「先发」的补取是 index 0，
 * 之后共享订阅为较新快照发出的请求排在其后。释放时显式给 index，不靠时间猜测。
 */
async function armAttentionGate(page) {
  await page.evaluate(`window.__attentionGate = window.__cotermTest.gate("host_attention")`);
}

/** 打开 A、等它真的弹出来，再武装闸门并取消——此时唯一的在途请求就是 finally 补取。 */
async function openCancelWithGatedReload(session, initialAttention) {
  const page = await openInteractionsPage(session, scenarioBase(initialAttention));
  await waitForDialogs(page, 1, "A opened");
  await flushPage(page);
  // 闸门必须在取消之前铺好，否则 finally 里的补取会直接走真实 handler。
  await armAttentionGate(page);
  await page.evaluate(`window.__cotermTest.clickCancel()`);
  await waitForValue({ send: page.send }, `window.__attentionGate.size() === 1`, { label: "post-cancel reload is in flight" });
  return page;
}

/** 让共享订阅先收敛到 next：它排在补取之后入队，所以按 index 1 放行。 */
async function deliverSnapshotAheadOfPrefetch(page, next) {
  await waitForValue({ send: page.send }, `window.__attentionGate.size() === 2`, { label: "shared subscription reload queued behind the prefetch" });
  await page.evaluate(`window.__attentionGate.release(${JSON.stringify(next)}, 1)`);
  await flushPage(page);
}

/** 再放行早先那次补取（index 0）。 */
async function releaseStalePrefetch(page, stale) {
  await page.evaluate(`window.__attentionGate.release(${JSON.stringify(stale)}, 0)`);
  await flushPage(page);
}

test("HostInteractions: a stale prefetch cannot reopen the request a newer snapshot removed", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; stale-prefetch verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    const page = await openCancelWithGatedReload(session, attention(1, [secretInteraction("int-a", "inst-a")]));
    try {
      await waitForDialogs(page, 0, "user cancel closes A");

      // 较新的空快照先到：共享订阅收敛到 v2，A 也从 settled 里清掉了。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, []))})`);
      await deliverSnapshotAheadOfPrefetch(page, attention(2, []));
      await waitForDialogs(page, 0, "the newer empty snapshot leaves zero dialogs");

      // 再放行早先发出的补取：它带回来的是旧版本 [A]。
      await releaseStalePrefetch(page, attention(1, [secretInteraction("int-a", "inst-a")]));
      assert.deepEqual(await dialogTitles(page), [], "a prefetch older than the delivered snapshot must not reopen A");
      assert.equal(
        await page.evaluate(`window.__cotermTest.pendingSecrets.length`),
        1,
        "only the cancel notification was delivered, nothing reopened and re-asked",
      );

      // 再推一跳，确认不是「还没轮到」而是真的不会被翻出来。
      await delay(200);
      assert.deepEqual(await dialogTitles(page), [], "A stays closed after the stale prefetch settles");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
  }
});

test("HostInteractions: a stale prefetch cannot displace the newer prompt, nor resurrect A after B drains", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; stale-prefetch vs newer prompt verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 180_000 });

  try {
    const page = await openCancelWithGatedReload(session, attention(1, [secretInteraction("int-a", "inst-a")]));
    try {
      await waitForDialogs(page, 0, "user cancel closes A");

      // 较新快照含 B：B 到达，A 已不在其中。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, [secretInteraction("int-b", "inst-b")]))})`);
      await deliverSnapshotAheadOfPrefetch(page, attention(2, [secretInteraction("int-b", "inst-b")]));
      await waitForDialogs(page, 1, "B opened from the newer snapshot");
      assert.match((await dialogTitles(page))[0], /inst-b/, "the newer prompt owns the dialog");

      // 旧补取回来（仍是 [A]）：既不能挤掉 B，也不能让 A 再出现。
      await releaseStalePrefetch(page, attention(1, [secretInteraction("int-a", "inst-a")]));
      let titles = await dialogTitles(page);
      assert.equal(titles.length, 1, "the stale prefetch must not add a second dialog");
      assert.match(titles[0], /inst-b/, "B is still the one prompting");

      // B 正常提交后归零（第 1 条是取消 A，第 2 条是 B 提交）。
      await page.evaluate(`window.__cotermTest.submitSecret("pw-for-b")`);
      await waitForValue({ send: page.send }, `window.__cotermTest.pendingSecrets.length === 2`, { label: "B submitted" });
      assert.equal((await page.evaluate(`window.__cotermTest.pendingSecrets[1]`)).interactionId, "int-b", "B's secret went to B");
      await waitForDialogs(page, 0, "queue drained to zero after B");

      // 归零之后旧 A 仍然不能回来：它比已收敛的版本旧。
      await delay(300);
      assert.deepEqual(await dialogTitles(page), [], "A must not reappear once the queue has drained");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
  }
});

test("HostInteractions: cancel, newer B, B cancelled — a stale A prefetch still reopens nothing", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; B cancel then stale-A verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 180_000 });

  try {
    const page = await openCancelWithGatedReload(session, attention(1, [secretInteraction("int-a", "inst-a")]));
    try {
      await waitForDialogs(page, 0, "user cancel closes A");

      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, [secretInteraction("int-b", "inst-b")]))})`);
      await deliverSnapshotAheadOfPrefetch(page, attention(2, [secretInteraction("int-b", "inst-b")]));
      await waitForDialogs(page, 1, "B opened");

      // B 也被用户取消：队列再次归零，B 的 finally 再发一次补取（队里排在旧补取之后）。
      await page.evaluate(`window.__cotermTest.clickCancel()`);
      await waitForValue({ send: page.send }, `window.__cotermTest.pendingSecrets.length === 2`, { label: "B cancel notified" });
      await waitForValue({ send: page.send }, `window.__attentionGate.size() === 2`, { label: "B's own reload queued" });
      await page.evaluate(`window.__attentionGate.release(${JSON.stringify(attention(2, []))}, 1)`);
      await flushPage(page);
      await waitForDialogs(page, 0, "cancelling B empties the queue");

      // 放行最早的补取：[A] 比已收敛的版本旧，仍然不能重开。
      await releaseStalePrefetch(page, attention(1, [secretInteraction("int-a", "inst-a")]));
      assert.deepEqual(await dialogTitles(page), [], "stale A prefetch reopens nothing after B was cancelled");
      await delay(200);
      assert.deepEqual(await dialogTitles(page), [], "still nothing after the queue settles");
      assert.equal(await page.evaluate(`window.__cotermTest.pendingSecrets.length`), 2, "no extra respond_secret beyond the two cancels");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
  }
});

test("HostInteractions: B can submit and drain before a stale A prefetch resolves", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; B submit then stale-A verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });
  try {
    const initial = attention(1, [secretInteraction("int-a", "inst-a")]);
    const page = await openCancelWithGatedReload(session, initial);
    try {
      const next = attention(2, [secretInteraction("int-b", "inst-b")]);
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(next)})`);
      await deliverSnapshotAheadOfPrefetch(page, next);
      assert.match((await dialogTitles(page))[0], /inst-b/, "B prompts while A's prefetch is still in flight");

      await page.evaluate(`window.__cotermTest.submitSecret("pw-for-b")`);
      await waitForValue({ send: page.send }, `window.__attentionGate.size() === 2`, { label: "B's own reload is in flight" });
      await page.evaluate(`window.__attentionGate.release(${JSON.stringify(attention(3, []))}, 1)`);
      await flushPage(page);
      assert.deepEqual(await dialogTitles(page), [], "B submits and the queue drains before A's reload returns");

      await releaseStalePrefetch(page, initial);
      assert.deepEqual(await dialogTitles(page), [], "late A stays closed after B submitted and drained");
      assert.deepEqual(
        await page.evaluate(`window.__cotermTest.pendingSecrets`),
        [{ interactionId: "int-a", value: null }, { interactionId: "int-b", value: "pw-for-b" }],
        "A's cancel and B's value stay bound to their own interactions",
      );
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
  }
});

test("HostInteractions: a prefetch or a failure landing after unmount is ignored", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; unmount-vs-late-reload verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    for (const completion of ["resolve", "reject"]) {
      const page = await openCancelWithGatedReload(session, attention(1, [secretInteraction("int-a", "inst-a")]));
      try {
        await waitForDialogs(page, 0, "user cancel closes A");

        // 卸载必须在补取还在途时发生，分别验证成功与失败回调。
        await page.evaluate("window.__cotermHostInteractions.unmount()");
        assert.equal(await page.evaluate(`window.__cotermTest.listenerCount()`), 0, "unmount removes the attention listener");

        if (completion === "resolve") {
          await page.evaluate(`window.__attentionGate.release(${JSON.stringify(attention(1, [secretInteraction("int-a", "inst-a")]))})`);
        } else {
          await page.evaluate(`window.__attentionGate.fail("reload failed after unmount")`);
        }
        await flushPage(page);
        assert.deepEqual(await dialogTitles(page), [], `a prefetch ${completion} after unmount opens nothing`);
        assert.equal(await page.evaluate(`document.querySelectorAll("[role=alert]").length`), 0, "no error rendered after unmount");
      } finally {
        await page.close();
      }
    }
  } finally {
    await session.close();
  }
});

test("HostInteractions: an older prefetch failure does not disturb the newer B prompt", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; stale-error verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    const page = await openCancelWithGatedReload(session, attention(1, [secretInteraction("int-a", "inst-a")]));
    try {
      await waitForDialogs(page, 0, "user cancel closes A");

      // 较新的快照先到（带 B），把补取彻底甩在身后。
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, [secretInteraction("int-b", "inst-b")]))})`);
      await deliverSnapshotAheadOfPrefetch(page, attention(2, [secretInteraction("int-b", "inst-b")]));
      await waitForDialogs(page, 1, "B opened");

      // 早先的补取以失败收场：它属于已经过去的时刻，不能改写当前状态。
      await page.evaluate(`window.__attentionGate.fail("stale reload failed", 0)`);
      await flushPage(page);
      const alerts = await page.evaluate(`[...document.querySelectorAll("[role=alert]")]
        .filter((node) => !node.closest("dialog.host-secret-dialog"))
        .map((node) => node.textContent)`);
      assert.deepEqual(alerts, [], `a stale reload failure must not surface over the newer state (saw ${JSON.stringify(alerts)})`);
      assert.match((await dialogTitles(page))[0], /inst-b/, "B is untouched by the stale failure");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
  }
});

test("HostInteractions: an older prefetch failure cannot replace a newer empty snapshot", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; empty-snapshot vs stale-error verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

  try {
    const page = await openCancelWithGatedReload(session, attention(1, [secretInteraction("int-a", "inst-a")]));
    try {
      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, []))})`);
      await deliverSnapshotAheadOfPrefetch(page, attention(2, []));
      assert.deepEqual(await dialogTitles(page), [], "newer empty snapshot closes the queue without opening B");

      await page.evaluate(`window.__attentionGate.fail("stale reload failed", 0)`);
      await flushPage(page);
      assert.deepEqual(
        await page.evaluate(`[...document.querySelectorAll("[role=alert]")].map((node) => node.textContent)`),
        [],
        "an older rejection must not add an error after the newer empty snapshot",
      );
      assert.deepEqual(await dialogTitles(page), [], "the stale rejection opens no dialog");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
  }
});


for (const completion of ["reject", "resolve"]) {
  test(`HostInteractions: an older prefetch ${completion} cannot overwrite a newer subscription error`, async (t) => {
    if (!browserPath) {
      t.skip("No Chrome/Edge binary found; subscription-error vs stale-error verification did not run");
      return;
    }
    assertFreshTestArtifacts();
    const { url, server } = await startStaticServer();

    t.after(() => new Promise((resolve) => server.close(resolve)));
    serverUrl = url;
    const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });

    try {
      const initial = attention(1, [secretInteraction("int-a", "inst-a"), secretInteraction("int-queued", "inst-queued")]);
      const page = await openCancelWithGatedReload(session, initial);
      try {
        // No newer prompt or snapshot is delivered, so only the subscription error can invalidate the reload.
        await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, []))})`);
        await waitForValue({ send: page.send }, `window.__attentionGate.size() === 2`, { label: "new subscription request is in flight" });
        await page.evaluate(`window.__attentionGate.fail("new subscription failed", 1)`);
        await flushPage(page);
        const alerts = () => page.evaluate(`[...document.querySelectorAll("[role=alert]")].map((node) => node.textContent)`);
        const expected = ["Unable to follow user interactions: new subscription failed"];
        assert.deepEqual(await alerts(), expected, "the newer subscription error is shown first");

        if (completion === "reject") {
          await page.evaluate(`window.__attentionGate.fail("stale reload failed", 0)`);
        } else {
          await page.evaluate(`window.__attentionGate.release(${JSON.stringify(initial)}, 0)`);
        }
        await flushPage(page);
        assert.deepEqual(await alerts(), expected, "the older completion must preserve the newer subscription error");
        assert.deepEqual(await dialogTitles(page), [], "the stale completion opens no dialog");

        await page.evaluate(`window.__cotermHostInteractions.retryHostAttention()`);
        await flushPage(page);
        assert.deepEqual(await alerts(), [], "a successful subscription retry clears the recoverable error");
        assert.deepEqual(await dialogTitles(page), [], "the recovered empty snapshot leaves the queue empty");
      } finally {
        await page.close();
      }
    } finally {
      await session.close();
    }
  });
}

test("HostInteractions: a current prefetch failure is visible and a newer snapshot recovers", async (t) => {
  if (!browserPath) {
    t.skip("No Chrome/Edge binary found; current-error recovery verification did not run");
    return;
  }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 120_000 });
  try {
    const page = await openCancelWithGatedReload(session, attention(1, [secretInteraction("int-a", "inst-a")]));
    try {
      await page.evaluate(`window.__attentionGate.fail("current reload failed")`);
      await flushPage(page);
      assert.deepEqual(
        await page.evaluate(`[...document.querySelectorAll("[role=alert]")].map((node) => node.textContent)`),
        ["Unable to reload user interactions: current reload failed"],
        "a current reload failure must remain visible",
      );

      await page.evaluate(`window.__cotermTest.setAttention(${JSON.stringify(attention(2, [secretInteraction("int-b", "inst-b")]))})`);
      await flushPage(page);
      assert.equal(await page.evaluate(`[...document.querySelectorAll("[role=alert]")].filter((node) => !node.closest("dialog.host-secret-dialog")).length`), 0, "a newer snapshot clears the error");
      const titles = await dialogTitles(page);
      assert.equal(titles.length, 1, "recovery opens exactly one prompt");
      assert.match(titles[0], /inst-b/, "recovery prompts B, not A");
    } finally {
      await page.close();
    }
  } finally {
    await session.close();
  }
});

test("Frontend SDK v1: typed host dialogs, rejection, timeout, scope replacement and disposal", async (t) => {
  if (!browserPath) { t.skip("No browser available for SDK dialog acceptance"); return; }
  assertFreshTestArtifacts();
  const { url, server } = await startStaticServer();

  t.after(() => new Promise((resolve) => server.close(resolve)));
  serverUrl = url;
  const session = await openSession({ browserPath, totalTimeoutMs: 60_000 });
  const page = await openInteractionsPage(session, scenarioBase(attention(1, [])));
  try {
    await page.evaluate(`window.api = window.__cotermHostInteractions.createPluginAPI({pluginId:'fixture',pluginInstanceId:'sdk-instance',sessionId:'session',label:'SDK',status:'ready'});
      window.result = null; window.api.requestText({title:'Text',prompt:'<b>plain text</b>'}).then(value=>window.result=value); true;`);
    await waitForDialogs(page, 1, "text prompt");
    assert.equal(await page.evaluate("document.querySelector('dialog label span').textContent"), "<b>plain text</b>");
    assert.equal(await page.evaluate("document.querySelector('dialog b')"), null);
    await page.evaluate("document.querySelector('dialog input').value='typed'; document.querySelector('dialog form').requestSubmit()");
    await waitForValue({send:page.send}, "window.result?.outcome === 'submitted'");
    assert.deepEqual(await page.evaluate("window.result"), {outcome:"submitted",value:"typed"});
    assert.equal(await page.evaluate("window.api.apiVersion"), "1.0");
    assert.equal(await page.evaluate("window.api.capabilities.streams"), false);

    await page.evaluate("window.result=null; window.api.requestSelection({title:'Select',prompt:'Choose',options:[{value:'a',label:'Alpha'},{value:'b',label:'Beta'}]}).then(value=>window.result=value); true");
    await waitForDialogs(page, 1, "selection prompt");
    await page.evaluate("document.querySelector('dialog select').value='b'; document.querySelector('dialog form').requestSubmit()");
    await waitForValue({send:page.send}, "window.result?.outcome === 'submitted'");
    assert.deepEqual(await page.evaluate("window.result"), {outcome:"submitted",value:"b"});

    await page.evaluate("window.result=null; window.api.requestConfirmation({title:'Confirm',prompt:'Proceed?'}).then(value=>window.result=value); true");
    await waitForDialogs(page, 1, "confirmation prompt");
    await page.evaluate("[...document.querySelectorAll('dialog button')].find(button=>button.textContent==='Reject').click()");
    assert.deepEqual(await page.evaluate("window.result"), {outcome:"rejected"});

    await page.evaluate("window.result=null; window.api.requestText({title:'Timeout',prompt:'Wait',timeoutMs:1000}).then(value=>window.result=value); true");
    await waitForValue({send:page.send}, "window.result?.outcome === 'timed_out'");
    await waitForDialogs(page, 0, "timeout removes dialog");
    await page.evaluate("window.oldResult=null; window.api.requestText({title:'Old',prompt:'Old'}).then(value=>window.oldResult=value); window.api.requestConfirmation({title:'New',prompt:'New',allowCancel:false}).then(value=>window.result=value); true");
    assert.deepEqual(await page.evaluate("window.oldResult"), {outcome:"cancelled"});
    await waitForDialogs(page, 1, "replacement does not stack");
    await page.evaluate("document.querySelector('dialog').dispatchEvent(new Event('cancel',{cancelable:true}))");
    await waitForDialogs(page, 1, "allowCancel false blocks escape");
    await page.evaluate("window.api.dispose()");
    await waitForDialogs(page, 0, "dispose removes prompt");
    assert.deepEqual(await page.evaluate("window.result"), {outcome:"cancelled"});
    assert.deepEqual(await page.evaluate("window.api.requestText({title:'Disposed',prompt:'No'})"), {outcome:"unavailable"});
    assert.equal(await page.evaluate("window.api.invoke('fixture.commit',{}).then(()=>false,()=>true)"), true);
  } finally {
    await page.close();
    await session.close();
    await new Promise(resolve=>server.close(resolve));
  }
});
