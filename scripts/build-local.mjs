import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const pnpm = process.env.npm_execpath;
if (!pnpm) throw new Error("Run this script with pnpm driver:local:build");
execFileSync(process.execPath, [pnpm, "--filter", "coterm-driver-local-ui", "build"], { cwd: root, stdio: "inherit" });
