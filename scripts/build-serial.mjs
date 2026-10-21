import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const pnpm = process.env.npm_execpath;
if (!pnpm) throw new Error("Run this script with pnpm plugin:serial:build");
execFileSync("cargo", ["build", "-p", "coterm-plugin-serial"], { cwd: root, stdio: "inherit" });
execFileSync(process.execPath, [pnpm, "--filter", "coterm-plugin-serial-ui", "build"], { cwd: root, stdio: "inherit" });
const output = path.join(root, "plugins/plugin-serial/dist");
const executable = `coterm-plugin-serial${process.platform === "win32" ? ".exe" : ""}`;
mkdirSync(path.join(output, "backend"), { recursive: true });
copyFileSync(path.join(root, "target/debug", executable), path.join(output, "backend", executable));
copyFileSync(path.join(root, "plugins/plugin-serial/manifest.json"), path.join(output, "manifest.json"));
console.log(`Serial development package: ${output}`);
