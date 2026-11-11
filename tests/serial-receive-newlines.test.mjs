import test from "node:test";
import assert from "node:assert/strict";
import { createReceiveNewlineNormalizer, readReceiveNewlineMode, saveReceiveNewlineMode,
  RECEIVE_NEWLINE_MODES } from "../crates/coterm-driver/frontend/serial/src/receive-newlines.ts";

const cases = [
  ["a\nb", ["a\nb", "a\r\nb", "a\nb", "a\r\nb"]],
  ["a\rb", ["a\rb", "a\rb", "a\r\nb", "a\r\nb"]],
  ["a\r\nb", ["a\r\nb", "a\r\nb", "a\r\nb", "a\r\nb"]],
  ["\n\n\r\r", ["\n\n\r\r", "\r\n\r\n\r\r", "\n\n\r\n\r\n", "\r\n\r\n\r\n\r\n"]],
  ["中文\u001b[31m红色\u001b[0m\t\0", Array(4).fill("中文\u001b[31m红色\u001b[0m\t\0")],
];

for (const [index, mode] of RECEIVE_NEWLINE_MODES.entries()) {
  test(`receive newlines: ${mode} preserves data and works across arbitrary packet splits`, () => {
    for (const [input, expected] of cases) {
      for (let split = 0; split <= input.length; split++) {
        const normalizer = createReceiveNewlineNormalizer(mode);
        assert.equal(normalizer.write(input.slice(0, split)) + normalizer.write("")
          + normalizer.write(input.slice(split)), expected[index], `split ${split} of ${JSON.stringify(input)}`);
      }
    }
    // Mixed endings, every possible partition (including consecutive one-byte packets).
    const input = "\r\nx\n\r\rZ\n";
    const expected = input.replace(/\r\n|\r|\n/g, ending => ending === "\r\n" ? ending
      : ending === "\r" && (mode === "cr" || mode === "both") ? "\r\n"
      : ending === "\n" && (mode === "lf" || mode === "both") ? "\r\n" : ending);
    for (let partition = 0; partition < 1 << (input.length - 1); partition++) {
      const normalizer = createReceiveNewlineNormalizer(mode);
      let result = "", start = 0;
      for (let i = 1; i < input.length; i++) {
        if (partition & 1 << (i - 1)) { result += normalizer.write(input.slice(start, i)); start = i; }
      }
      result += normalizer.write(input.slice(start));
      assert.equal(result, expected);
    }
  });
}

test("CR completion is immediate; reset separates clear, reconnect and replay gaps", () => {
  const normalizer = createReceiveNewlineNormalizer("both");
  assert.equal(normalizer.write("\r"), "\r\n", "a lone CR does not wait for the next event");
  assert.equal(normalizer.write(""), "");
  assert.equal(normalizer.write("\n"), "", "the original LF does not create a second line");
  normalizer.write("\r");
  normalizer.reset();
  assert.equal(normalizer.write("\n"), "\r\n", "a new stream cannot inherit the previous CR");
});

test("newline preferences are per instance and tolerate invalid or unavailable storage", () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  const values = new Map();
  try {
    Object.defineProperty(globalThis, "localStorage", { configurable: true, value: {
      getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value),
    } });
    assert.equal(readReceiveNewlineMode("one"), "none");
    saveReceiveNewlineMode("one", "lf");
    assert.equal(readReceiveNewlineMode("one"), "lf");
    assert.equal(readReceiveNewlineMode("two"), "none");
    values.set("coterm.serial.receive-newlines.v1:one", "invalid");
    assert.equal(readReceiveNewlineMode("one"), "none");
    Object.defineProperty(globalThis, "localStorage", { configurable: true, get() { throw new Error("denied"); } });
    assert.equal(readReceiveNewlineMode("one"), "none");
    assert.doesNotThrow(() => saveReceiveNewlineMode("one", "both"));
  } finally {
    if (previous) Object.defineProperty(globalThis, "localStorage", previous);
    else delete globalThis.localStorage;
  }
});
