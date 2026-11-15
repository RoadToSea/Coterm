import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";

test("built-in drivers are linked libraries with declarative host views and no worker artifacts", () => {
  const composition = fs.readFileSync(new URL("../../apps/desktop/src-tauri/src/builtin_drivers.rs", import.meta.url), "utf8");
  const dependencies = fs.readFileSync(new URL("../../apps/desktop/src-tauri/Cargo.toml", import.meta.url), "utf8");
  assert.ok(dependencies.includes("coterm-driver.workspace = true"));
  const base = new URL("../../crates/coterm-driver/", import.meta.url);
  assert.equal(fs.existsSync(new URL("Cargo.toml", base)), true);
  for (const name of ["ssh", "serial", "local", "tcp", "telnet"]) {
    const manifest = JSON.parse(fs.readFileSync(new URL(`manifests/${name}.json`, base)));
    assert.equal(manifest.id, `coterm-driver-${name}`);
    assert.equal(Object.hasOwn(manifest, "backend"), false);
    assert.equal(fs.existsSync(new URL(`src/${name}/mod.rs`, base)), true);
    assert.equal(fs.existsSync(new URL(`src/${name}/Cargo.toml`, base)), false);
    assert.ok(composition.includes(`register::<coterm_driver::${name}::backend::Backend>`));
    assert.equal(manifest.frontend, undefined);
    assert.ok(["terminal", "stream"].includes(manifest.ui.workbench));
    assert.equal(manifest.ui.schema.type, "object");
    assert.ok(Object.keys(manifest.ui.schema.properties).length > 0);

  }
  const config = JSON.parse(fs.readFileSync(new URL("../../apps/desktop/src-tauri/tauri.conf.json", import.meta.url)));
  assert.equal(config.bundle.resources, undefined);
});
