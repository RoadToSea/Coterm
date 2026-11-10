/**
 * CLI: pnpm debug:clean [--dry-run]. No path arguments are accepted.
 * Prints the number of entries removed and retains .agent-debug/ itself.
 * Exits with code 1 for invalid arguments, an unsafe root, or filesystem errors.
 */
import { lstat, mkdir, readdir, realpath, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const debugDirectory = path.join(repositoryRoot, ".agent-debug");
const usage = "Usage: pnpm debug:clean [--dry-run]";

async function validateDirectory(create) {
  let info;
  try {
    info = await lstat(debugDirectory);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    if (!create) return false;
    await mkdir(debugDirectory);
    info = await lstat(debugDirectory);
  }
  if (info.isSymbolicLink() || !info.isDirectory()) {
    throw new Error(".agent-debug must be a real directory, not a file, symbolic link, or junction");
  }
  const expected = path.join(await realpath(repositoryRoot), ".agent-debug");
  if (path.relative(expected, await realpath(debugDirectory)) !== "") {
    throw new Error(".agent-debug resolves outside the repository debug directory");
  }
  return true;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.some(arg => !["--dry-run", "--help"].includes(arg))) throw new Error(usage);
  if (args.includes("--help")) {
    console.log(usage);
    return;
  }
  const dryRun = args.includes("--dry-run");
  if (!await validateDirectory(!dryRun)) {
    console.log(".agent-debug/ does not exist; nothing to clean");
    return;
  }
  const entries = (await readdir(debugDirectory)).sort();
  if (dryRun) {
    console.log(JSON.stringify({ directory: ".agent-debug/", entries, dryRun: true }, null, 2));
    return;
  }
  for (const entry of entries) {
    await validateDirectory(false);
    await rm(path.join(debugDirectory, entry), {
      recursive: true, force: true, maxRetries: 2, retryDelay: 100,
    });
  }
  console.log(`Cleaned .agent-debug/: ${entries.length} entries removed; directory retained`);
}

main().catch(error => {
  console.error(`Debug cleanup failed: ${error.message}`);
  process.exitCode = 1;
});
