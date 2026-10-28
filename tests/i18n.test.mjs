import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { getLocale, resolveLocale, setLocale, subscribeLocale, t, LOCALE_STORAGE_KEY } from "../apps/desktop/src/i18n.ts";
import { en } from "../apps/desktop/src/locales/en.ts";
import { zhCN } from "../apps/desktop/src/locales/zh-CN.ts";
import { createTranslator } from "../plugins/plugin-ssh/frontend/src/i18n.ts";

test("language selection honors a saved preference and otherwise follows the system language", () => {
  assert.equal(resolveLocale("en", ["zh-CN"]), "en");
  assert.equal(resolveLocale("zh-CN", ["en-US"]), "zh-CN");
  assert.equal(resolveLocale(null, ["zh-HK", "en-US"]), "zh-CN");
  assert.equal(resolveLocale("bad", ["en-US", "zh-CN"]), "en");
  assert.equal(resolveLocale(null, ["fr"]), "en");
  assert.equal(resolveLocale(null, []), "en");
});

test("switching languages translates host text, interpolates values, and persists the choice", () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  const values = new Map();
  Object.defineProperty(globalThis, "localStorage", { configurable: true, value: { setItem: (key, value) => values.set(key, value) } });
  try {
    setLocale("en");
    let updates = 0;
    const stop = subscribeLocale(() => updates++);
    setLocale("zh-CN");
    assert.equal(getLocale(), "zh-CN");
    assert.equal(t("Install plugin"), "安装插件");
    assert.equal(t("Manage {name}", { name: "SSH" }), "管理 SSH");
    assert.equal(t("vendor-owned text"), "vendor-owned text");
    assert.equal(values.get(LOCALE_STORAGE_KEY), "zh-CN");
    setLocale("zh-CN");
    assert.equal(updates, 1, "the same language does not notify listeners");
    stop();
    setLocale("en");
    assert.equal(t("Install plugin"), "Install plugin");
    assert.equal(updates, 1, "released subscribers stay released");
  } finally {
    if (original) Object.defineProperty(globalThis, "localStorage", original);
    else delete globalThis.localStorage;
    setLocale("en");
  }
});

test("disabled storage and an exception in a plugin listener do not prevent language switching", () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", { configurable: true, get() { throw new Error("storage disabled"); } });
  const stopBad = subscribeLocale(() => { throw new Error("plugin listener failed"); });
  let updated = false;
  const stopGood = subscribeLocale(() => { updated = true; });
  try {
    setLocale("zh-CN");
    assert.equal(updated, true);
    assert.equal(t("Cancel"), "取消");
    setLocale("invalid");
    assert.equal(getLocale(), "zh-CN");
  } finally {
    stopBad(); stopGood();
    if (original) Object.defineProperty(globalThis, "localStorage", original);
    else delete globalThis.localStorage;
    setLocale("en");
  }
});

test("SSH uses the optional SDK language capability and falls back on older hosts", () => {
  let locale = "zh-CN";
  const translate = createTranslator({ get locale() { return locale; }, subscribe: () => () => {} });
  assert.equal(translate("Connect"), "连接");
  assert.equal(translate("Password for {server}", { server: "ops@server" }), "ops@server 的密码");
  locale = "en";
  assert.equal(translate("Connect"), "Connect");
  assert.equal(createTranslator()("Connect"), "Connect");
});

test("host error codes have messages in both languages and host UI keys have Chinese translations", () => {
  for (const code of Object.keys(en)) assert.ok(zhCN[code], `missing Chinese error ${code}`);
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const path = new URL(`${entry.name}${entry.isDirectory() ? "/" : ""}`, directory);
      if (entry.isDirectory()) visit(path);
      else if (/\.tsx$/.test(entry.name)) {
        const source = fs.readFileSync(path, "utf8");
        for (const match of source.matchAll(/\bt\("([^"\n]+)"/g)) {
          assert.ok(zhCN[match[1]] || en[match[1]], `missing Chinese translation ${match[1]} in ${entry.name}`);
        }
      }
    }
  };
  visit(new URL("../apps/desktop/src/", import.meta.url));
});
