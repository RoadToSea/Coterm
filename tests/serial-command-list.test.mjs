import test from "node:test";
import assert from "node:assert/strict";
import { commandList, waitForCommands } from "../plugins/plugin-serial/frontend/src/command-list.ts";
import { createTranslator } from "../plugins/plugin-serial/frontend/src/i18n.ts";

test("shell editor builds a list without composing shell commands", () => {
  assert.deepEqual(commandList("cd /tmp\r\n\r\nprintf 'a && b'\n  echo 中文\n", "cr", 100), {
    commands: ["cd /tmp", "printf 'a && b'", "  echo 中文"], lineEnding: "cr", intervalMs: 100,
  });
  assert.equal(commandList(Array(4).fill("x".repeat(4095)).join("\n"), "lf", 0).commands.length, 4);
});

test("editor rejects invalid list sizes, embedded controls and UTF-8 byte overflow", () => {
  for (const args of [
    ["\n \n", "cr", 100], [Array(65).fill("pwd").join("\n"), "cr", 0],
    ["echo\rreboot", "cr", 100], ["\u001b[A", "cr", 100],
    ["中".repeat(1366), "lf", 100], [Array(4).fill("x".repeat(4096)).join("\n"), "lf", 0],
    ["pwd", "none", 100], ["pwd", "cr", -1], ["pwd", "cr", 1001], ["pwd", "cr", 1.5],
  ]) assert.throws(() => commandList(...args));
});

const receipt = (status, output = null) => ({ operationId: "one", status, output, error: null, waitExpired: false, proposalId: null });

test("waiting uses the host terminal state even if completion preceded the invoke response", async () => {
  let calls = 0;
  const api = { getOperation: async (id) => { calls++; assert.equal(id, "one"); return receipt("completed", { sent: 2, total: 2 }); } };
  const result = await waitForCommands(api, receipt("running"), new AbortController().signal);
  assert.equal(result.output.sent, 2);
  assert.equal(calls, 1);
  const finished = receipt("cancelled");
  assert.equal(await waitForCommands(api, finished, new AbortController().signal), finished);
  assert.equal(calls, 1);
});

test("unmount aborts the wait without sending a new operation or cancelling device work", async () => {
  const controller = new AbortController();
  const api = { getOperation: async () => { assert.fail("aborted wait must not poll"); } };
  const pending = waitForCommands(api, receipt("running"), controller.signal);
  controller.abort();
  await assert.rejects(pending, /closed/);
});

test("new queue labels have Chinese translations", () => {
  const t = createTranslator({ locale: "zh-CN", subscribe: () => () => {} });
  for (const key of ["Commands (one per line)", "Send list", "Stop queue", "Interval (ms)"])
    assert.notEqual(t(key), key);
  assert.equal(t("Sent {sent}/{total}; device completion is unknown.", { sent: 1, total: 3 }), "已发送 1/3 条；设备执行是否完成仍未知。");
});
