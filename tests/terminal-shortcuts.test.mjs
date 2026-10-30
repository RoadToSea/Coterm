import test from "node:test";
import assert from "node:assert/strict";
import { attachTerminalShortcuts } from "../packages/plugin-sdk/src/components/terminal-shortcuts.ts";

function fixture({ selection = "selected output", clipboardFails = false, nativeCopy = false } = {}) {
  const copies = [];
  const listeners = new Map();
  let errors = 0;
  let nativeCalls = 0;
  const terminal = {
    element: {
      matches: () => true,
      ownerDocument: {
        defaultView: { navigator: { clipboard: { async writeText(text) {
          if (clipboardFails) throw new Error("permission denied");
          copies.push(text);
        } } } },
        execCommand(command) { assert.equal(command, "copy"); nativeCalls++; return nativeCopy; },
      },
      addEventListener(name, fn) { listeners.set(name, fn); },
      removeEventListener(name, fn) { assert.equal(listeners.get(name), fn); listeners.delete(name); },
    },
    hasSelection: () => selection.length > 0,
    getSelection: () => selection,
    clearSelection: () => { selection = ""; },
    selectAll: () => { selection = "all output"; },
    attachCustomKeyEventHandler(fn) { this.handler = fn; },
  };
  const dispose = attachTerminalShortcuts(terminal, () => errors++);
  const press = (key, flags = {}) => {
    const event = {
      key, code:key, type:"keydown", ctrlKey:false, shiftKey:false, metaKey:false,
      altKey:false, isComposing:false, repeat:false, keyCode:0, prevented:false, stopped:false,
      preventDefault() { this.prevented = true; }, stopPropagation() { this.stopped = true; },
      ...flags,
    };
    return { forwarded:terminal.handler(event), event };
  };
  const release = (key, flags = {}) => press(key, { ...flags, type:"keyup" });
  return { terminal, copies, listeners, press, release, dispose,
    get errors() { return errors; }, get nativeCalls() { return nativeCalls; } };
}

test("Ctrl+C copies only with a terminal selection; explicit copy shortcuts never interrupt", async () => {
  const f = fixture();
  assert.equal(f.press("c", { ctrlKey:true }).forwarded, false);
  assert.equal(f.release("c", { ctrlKey:true }).forwarded, false);
  await Promise.resolve();
  assert.deepEqual(f.copies, ["selected output"]);
  assert.equal(f.terminal.hasSelection(), true, "copy retains the selection until Escape or a new selection");
  f.terminal.clearSelection();
  assert.equal(f.press("c", { ctrlKey:true }).forwarded, true, "unselected Ctrl+C reaches the shell");
  assert.equal(f.press("C", { ctrlKey:true, shiftKey:true }).forwarded, false);
  assert.equal(f.press("Insert", { ctrlKey:true }).forwarded, false);
  assert.equal(f.press("c", { metaKey:true }).forwarded, false);
  assert.deepEqual(f.copies, ["selected output"], "no selection means explicit copy is a local no-op");
  f.dispose();
});

test("paste shortcuts keep the native paste event and never become raw Ctrl+V", () => {
  for (const [key, flags] of [
    ["v", {ctrlKey:true}], ["V", {ctrlKey:true,shiftKey:true}],
    ["Insert", {shiftKey:true}], ["v", {metaKey:true}],
  ]) {
    const f = fixture();
    const { forwarded, event } = f.press(key, flags);
    assert.equal(forwarded, false);
    assert.equal(event.prevented, false, "the browser must still dispatch its native paste event");
    assert.equal(event.stopped, true);
    assert.equal(f.release(key, flags).forwarded, false);
    assert.deepEqual(f.copies, []);
    f.dispose();
  }
});

test("select-all and Escape act locally, while bare Ctrl+A and unselected Escape reach the shell", () => {
  const f = fixture({selection:""});
  assert.equal(f.press("a", {ctrlKey:true}).forwarded, true);
  for (const flags of [{ctrlKey:true,shiftKey:true}, {metaKey:true}]) {
    assert.equal(f.press("a", flags).forwarded, false);
    assert.equal(f.terminal.getSelection(), "all output");
    f.release("a", flags);
    assert.equal(f.press("Escape").forwarded, false);
    assert.equal(f.terminal.hasSelection(), false);
    f.release("Escape");
  }
  assert.equal(f.press("Escape").forwarded, true);
  f.dispose();
});

test("shell editing, IME and AltGr keep their existing behavior", () => {
  const f = fixture();
  for (const key of ["a", "e", "r", "d", "l", "u", "w", "z"]) {
    assert.equal(f.press(key, {ctrlKey:true}).forwarded, true, `Ctrl+${key}`);
  }
  for (const key of ["Enter", "Tab", "ArrowUp", "ArrowLeft", "Home", "End", "PageUp", "Backspace"])
    assert.equal(f.press(key).forwarded, true, key);
  assert.equal(f.press("c", {ctrlKey:true,altKey:true}).forwarded, true);
  assert.equal(f.press("c", {ctrlKey:true,isComposing:true}).forwarded, true);
  assert.equal(f.press("c", {ctrlKey:true,keyCode:229}).forwarded, true);
  f.dispose();
});

test("a local key gesture stays local through repeats, and focus loss releases held keys", async () => {
  const f = fixture();
  f.press("c", {ctrlKey:true});
  f.terminal.clearSelection();
  assert.equal(f.press("c", {ctrlKey:true,repeat:true}).forwarded, false);
  assert.equal(f.release("c", {ctrlKey:true}).forwarded, false);
  assert.equal(f.press("c", {ctrlKey:true}).forwarded, true);
  f.terminal.selectAll();
  f.press("c", {ctrlKey:true});
  f.listeners.get("focusout")();
  assert.equal(f.press("c").forwarded, true, "a lost keyup cannot swallow the next ordinary character");
  await Promise.resolve();
  assert.equal(f.copies.length, 2);
  f.dispose();
});

test("clipboard denial falls back to native copy; total failure reports an error without interrupting", async () => {
  for (const nativeCopy of [false, true]) {
    const f = fixture({clipboardFails:true,nativeCopy});
    assert.equal(f.press("c", {ctrlKey:true}).forwarded, false);
    await Promise.resolve();
    assert.equal(f.nativeCalls, 1);
    assert.equal(f.errors, nativeCopy ? 0 : 1);
    f.dispose();
  }
});

test("disposal removes listeners and ignores late clipboard failures", async () => {
  const f = fixture({clipboardFails:true});
  f.press("c", {ctrlKey:true});
  f.dispose();
  await Promise.resolve();
  assert.equal(f.errors, 0);
  assert.equal(f.nativeCalls, 0);
  assert.equal(f.listeners.size, 0);
  assert.equal(f.press("c", {ctrlKey:true}).forwarded, true);
});

test("a late clipboard denial cannot copy from a different focus or selection", async () => {
  for (const change of [f => { f.terminal.element.matches = () => false; }, f => f.terminal.selectAll()]) {
    const f = fixture({clipboardFails:true,nativeCopy:true});
    f.press("c", {ctrlKey:true});
    change(f);
    await Promise.resolve();
    assert.equal(f.nativeCalls, 0);
    assert.equal(f.errors, 1);
    f.dispose();
  }
});
