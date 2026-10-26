import { run } from "@tauri-apps/cli";
import { fileURLToPath } from "node:url";

// A direct CLI call preserves Cargo's argument separator across pnpm wrappers.
process.chdir(fileURLToPath(new URL("../apps/desktop/", import.meta.url)));
await run(["build", ...process.argv.slice(2), "--", "--locked"], "pnpm tauri:build");
