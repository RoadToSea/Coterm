import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { writePlatformManifest } from "./driver-package.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const pnpm = process.env.npm_execpath;
if (!pnpm) throw new Error("Run this script with pnpm driver:serial:build");
execFileSync("cargo", ["build", "-p", "coterm-driver-serial"], { cwd: root, stdio: "inherit" });
execFileSync(process.execPath, [pnpm, "--filter", "coterm-driver-serial-ui", "build"], { cwd: root, stdio: "inherit" });
const output = path.join(root, "drivers/driver-serial/dist");
const executable = `coterm-driver-serial${process.platform === "win32" ? ".exe" : ""}`;
mkdirSync(path.join(output, "backend"), { recursive: true });
copyFileSync(path.join(root, "target/debug", executable), path.join(output, "backend", executable));
writePlatformManifest(path.join(root, "drivers/driver-serial/manifest.json"), path.join(output, "manifest.json"));
console.log(`Serial development package: ${output}`);
