import test from "node:test";
import assert from "node:assert/strict";
import { getThemePreference, resolveThemePreference, resolveTheme, setThemePreference, subscribeTheme, THEME_STORAGE_KEY } from "../apps/desktop/src/theme.ts";

test("appearance follows the system unless a valid explicit choice is saved", () => {
  for (const invalid of [null, "", "blue", "SYSTEM"]) assert.equal(resolveThemePreference(invalid), "system");
  for (const mode of ["light", "dark"]) assert.equal(resolveThemePreference(mode), mode);
  assert.equal(resolveTheme("system", true), "dark");
  assert.equal(resolveTheme("system", false), "light");
  assert.equal(resolveTheme("light", true), "light");
  assert.equal(resolveTheme("dark", false), "dark");
});

test("appearance persists without notifying on repeated choices and tolerates unavailable storage", () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  const values = new Map();
  let updates = 0;
  setThemePreference("system");
  const stop = subscribeTheme(() => updates++);
  try {
    Object.defineProperty(globalThis, "localStorage", { configurable: true, value: { setItem: (key, value) => values.set(key, value) } });
    setThemePreference("dark");
    setThemePreference("dark");
    assert.equal(getThemePreference(), "dark");
    assert.equal(values.get(THEME_STORAGE_KEY), "dark");
    assert.equal(updates, 1);
    setThemePreference("invalid");
    assert.equal(getThemePreference(), "dark");
    Object.defineProperty(globalThis, "localStorage", { configurable: true, get() { throw new Error("storage disabled"); } });
    setThemePreference("light");
    assert.equal(getThemePreference(), "light");
    assert.equal(updates, 2);
    stop();
    setThemePreference("system");
    assert.equal(updates, 2);
  } finally {
    stop();
    if (original) Object.defineProperty(globalThis, "localStorage", original);
    else delete globalThis.localStorage;
    setThemePreference("system");
  }
});
