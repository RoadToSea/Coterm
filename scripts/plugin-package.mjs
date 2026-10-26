import { readFileSync, writeFileSync } from "node:fs";

/** A development build contains one native binary, so advertise exactly that target. */
export function platformManifest(manifest, os = process.platform, arch = process.arch) {
  const name = { win32: "windows", darwin: "macos", linux: "linux" }[os];
  if (!name || !["x64", "arm64"].includes(arch)) throw new Error("Unsupported plugin build target");
  const target = `${name}-${arch}`;
  const entry = manifest.backend[target];
  if (typeof entry !== "string") throw new Error(`Plugin does not declare ${target}`);
  return { ...manifest, backend: { [target]: entry } };
}
export function writePlatformManifest(source, destination) {
  writeFileSync(destination, JSON.stringify(platformManifest(JSON.parse(readFileSync(source, "utf8"))), null, 2) + "\n");
}
