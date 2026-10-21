import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const pnpm = process.env.npm_execpath;
if (!pnpm) throw new Error("Run this script with pnpm plugin:ssh:build");
execFileSync("cargo", ["build", "-p", "coterm-plugin-ssh"], { cwd: root, stdio: "inherit" });
execFileSync(process.execPath, [pnpm, "--filter", "coterm-plugin-ssh-ui", "build"], { cwd: root, stdio: "inherit" });
const output = path.join(root, "plugins/plugin-ssh/dist");
const executable = `coterm-plugin-ssh${process.platform === "win32" ? ".exe" : ""}`;
mkdirSync(path.join(output, "backend"), { recursive: true });
copyFileSync(path.join(root, "target/debug", executable), path.join(output, "backend", executable));
copyFileSync(path.join(root, "plugins/plugin-ssh/manifest.json"), path.join(output, "manifest.json"));
console.log(`SSH development package: ${output}`);
