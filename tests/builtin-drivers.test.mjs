import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";

test("built-in drivers are linked libraries with embedded frontends and no worker artifacts", () => {
  const composition = fs.readFileSync(new URL("../apps/desktop/src-tauri/src/builtin_drivers.rs", import.meta.url), "utf8");
  const dependencies = fs.readFileSync(new URL("../apps/desktop/src-tauri/Cargo.toml", import.meta.url), "utf8");
  for (const name of ["ssh", "serial", "local"]) {
    const base = new URL(`../drivers/driver-${name}/`, import.meta.url);
    const manifest = JSON.parse(fs.readFileSync(new URL("manifest.json", base)));
    assert.equal(manifest.id, `coterm-driver-${name}`);
    assert.equal(Object.hasOwn(manifest, "backend"), false);
    assert.equal(fs.existsSync(new URL("src/lib.rs", base)), true);
    assert.equal(fs.existsSync(new URL("src/main.rs", base)), false);
    assert.ok(dependencies.includes(`coterm-driver-${name} = { path =`));
    assert.ok(composition.includes(`register::<coterm_driver_${name}::backend::Backend>`));
    assert.ok(composition.includes(`drivers/driver-${name}/dist/frontend/index.js`));
    const build = fs.readFileSync(new URL(`../scripts/build-${name}.mjs`, import.meta.url), "utf8");
    assert.doesNotMatch(build, /copyFileSync|"cargo"|\.exe\b|writePlatformManifest/);
  }
  const config = JSON.parse(fs.readFileSync(new URL("../apps/desktop/src-tauri/tauri.conf.json", import.meta.url)));
  assert.equal(config.bundle.resources, undefined);
});
