import test from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_SERIAL_DISPLAY_PREFERENCES as defaults, readSerialDisplayPreferences as read,
  saveSerialDisplayPreferences as save } from "../drivers/driver-serial/frontend/src/display-preferences.ts";

function withStorage(run) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  const values = new Map();
  try {
    Object.defineProperty(globalThis, "localStorage", { configurable: true, value: {
      getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value),
    } });
    run(values);
  } finally {
    if (previous) Object.defineProperty(globalThis, "localStorage", previous);
    else delete globalThis.localStorage;
  }
}

const preferences = { view: "uart", receiveFormat: "hex", timestamps: true, autoScroll: false,
  receiveNewlines: "both", backspace: "del", sendLineEnding: "cr" };

test("display choices belong to a saved reference across recreated runtime IDs", () => withStorage(() => {
  save("ref-one", preferences, "old-runtime");
  assert.deepEqual(read("ref-one", "new-runtime"), preferences);
  assert.deepEqual(read("ref-two", "new-runtime"), defaults);
  save("ref-two", { ...preferences, receiveNewlines: "lf" }, "another-runtime");
  assert.deepEqual(read("ref-one"), preferences);
  assert.equal(read("ref-two").receiveNewlines, "lf");
}));

test("older hosts without saved identity keep runtime-local choices without guessing a device or name", () => withStorage(() => {
  save(undefined, preferences, "old-runtime");
  assert.deepEqual(read(undefined, "old-runtime"), preferences);
  assert.deepEqual(read(undefined, "new-runtime"), defaults);
  assert.deepEqual(read("old-runtime"), defaults, "config references and runtime IDs have separate namespaces");
  assert.deepEqual(read(), defaults);
  assert.doesNotThrow(() => save(undefined, preferences));
}));

test("display storage is bounded, validates fields and excludes output and executable state", () => withStorage(values => {
  save("ref-one", { ...preferences, command: "secret", repeat: true, output: "private", sendFormat: "hex", repeatMs: 250 });
  const key = "coterm.serial.display.v1:config:ref-one";
  assert.deepEqual(JSON.parse(values.get(key)), { version: 1, ...preferences });
  values.set(key, JSON.stringify({ version: 1, view: "invalid", timestamps: "true", autoScroll: 0,
    receiveNewlines: "invalid", backspace: "invalid", receiveFormat: "hex" }));
  assert.deepEqual(read("ref-one"), { ...defaults, receiveFormat: "hex" });
  for (const invalid of ["{", "null", "[]", '{"version":2}', " ".repeat(2049)]) {
    values.set(key, invalid);
    assert.deepEqual(read("ref-one"), defaults);
  }
  values.clear();
  for (const invalidRef of ["", " ", "x".repeat(513), "ref\u0000", "ref\u007f"]) {
    save(invalidRef, preferences);
    assert.deepEqual(read(invalidRef), defaults);
  }
  assert.equal(values.size, 0);
}));

test("legacy newline preference migrates only from the known current runtime instance", () => withStorage(values => {
  values.set("coterm.serial.receive-newlines.v1:old-runtime", "both");
  assert.deepEqual(read("ref-one", "new-runtime"), defaults, "do not guess another instance's settings");
  assert.deepEqual(read("ref-one", "old-runtime"), { ...defaults, receiveNewlines: "both" });
  assert.deepEqual(read("ref-one", "new-runtime"), { ...defaults, receiveNewlines: "both" });
  save("ref-one", preferences);
  assert.deepEqual(read("ref-one", "old-runtime"), preferences, "new format takes precedence");
}));

test("send line endings restore independently and older preferences keep the CRLF default", () => withStorage(values => {
  const key = "coterm.serial.display.v1:config:ref-one";
  values.set(key, JSON.stringify({ version: 1, view: "uart" }));
  assert.equal(read("ref-one").sendLineEnding, "crlf");
  for (const sendLineEnding of ["none", "cr", "lf", "crlf"]) {
    save("ref-one", { ...preferences, sendLineEnding }, "old-runtime");
    assert.equal(read("ref-one", "new-runtime").sendLineEnding, sendLineEnding);
    assert.equal(read("ref-two").sendLineEnding, "crlf");
  }
  values.set(key, JSON.stringify({ version: 1, sendLineEnding: "invalid" }));
  assert.equal(read("ref-one").sendLineEnding, "crlf");
}));

test("unavailable or full storage does not break display controls", () => withStorage(() => {
  Object.defineProperty(globalThis, "localStorage", { configurable: true, get() { throw new Error("denied"); } });
  assert.deepEqual(read("ref-one"), defaults);
  assert.doesNotThrow(() => save("ref-one", preferences));
  Object.defineProperty(globalThis, "localStorage", { configurable: true, value: {
    getItem: () => null, setItem() { throw new Error("quota exceeded"); },
  } });
  assert.doesNotThrow(() => save("ref-one", preferences));
}));
