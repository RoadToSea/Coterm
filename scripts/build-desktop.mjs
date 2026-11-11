import { run } from "@tauri-apps/cli";
import { fileURLToPath } from "node:url";

// A direct CLI call preserves Cargo's argument separator across pnpm wrappers.
process.chdir(fileURLToPath(new URL("../apps/desktop/", import.meta.url)));
const args = process.argv.slice(2).filter((arg) => arg !== "--release");
await run(["build", ...args, "--", "--locked"], "pnpm tauri:build");
