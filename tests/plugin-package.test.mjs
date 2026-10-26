import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { platformManifest } from "../scripts/plugin-package.mjs";

test("native plugin packages declare only the platform whose binary was built", () => {
  for (const plugin of ["ssh", "serial"]) {
    const source = JSON.parse(fs.readFileSync(new URL(`../plugins/plugin-${plugin}/manifest.json`, import.meta.url)));
    for (const [os, platform] of [["win32", "windows"], ["linux", "linux"], ["darwin", "macos"]]) {
      for (const arch of ["x64", "arm64"]) {
        const built = platformManifest(source, os, arch);
        assert.deepEqual(Object.keys(built.backend), [`${platform}-${arch}`]);
        assert.equal(built.backend[`${platform}-${arch}`], source.backend[`${platform}-${arch}`]);
        assert.equal(built.id, source.id);
      }
    }
    assert.throws(() => platformManifest(source, "win32", "ia32"));
  }
});
