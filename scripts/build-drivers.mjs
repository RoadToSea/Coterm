import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const pnpm = process.env.npm_execpath;
if (!pnpm) throw new Error("Run with pnpm drivers:build");
for (const name of ["ssh", "serial", "local"]) {
  execFileSync(process.execPath, [pnpm, `driver:${name}:build`], { cwd: root, stdio: "inherit" });
}
