import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const pnpm = process.env.npm_execpath;
if (!pnpm) throw new Error("Run with pnpm drivers:build");
for (const name of ["ssh", "serial", "local", "tcp", "telnet"]) {
  execFileSync(process.execPath, [pnpm, "--filter", `coterm-driver-${name}-ui`, "build"], { cwd: root, stdio: "inherit" });
}
