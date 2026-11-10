import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "coterm-debug-cleanup-"));
  const repository = path.join(directory, "repository");
  const script = path.join(repository, "scripts", "clean-agent-debug.mjs");
  fs.mkdirSync(path.dirname(script), { recursive: true });
  fs.copyFileSync(new URL("../scripts/clean-agent-debug.mjs", import.meta.url), script);
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return {
    directory, repository, debug: path.join(repository, ".agent-debug"),
    run(args = []) {
      const result = spawnSync(process.execPath, [script, ...args], {
        cwd: directory, encoding: "utf8", timeout: 10_000,
      });
      assert.ifError(result.error);
      return result;
    },
  };
}

function write(file, content = "keep") {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
}

test("cleanup removes nested and hidden artifacts, retains its root, and preserves repository files", t => {
  const f = fixture(t);
  write(path.join(f.debug, "session", "nested", "trace.log"), "debug");
  write(path.join(f.debug, ".hidden"), "debug");
  const preserved = ["target/debug/cache", "target/release/cache", "node_modules/package", "src/main.rs"];
  for (const file of preserved) write(path.join(f.repository, file));
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(fs.readdirSync(f.debug), []);
  assert.match(result.stdout, /2 entries removed/);
  for (const file of preserved) assert.equal(fs.readFileSync(path.join(f.repository, file), "utf8"), "keep");
});

test("cleanup creates a missing directory and repeated cleanup is harmless", t => {
  const f = fixture(t);
  assert.equal(f.run().status, 0);
  assert.deepEqual(fs.readdirSync(f.debug), []);
  assert.equal(f.run().status, 0);
  assert.deepEqual(fs.readdirSync(f.debug), []);
});

test("dry-run lists entries without modifying their contents", t => {
  const f = fixture(t);
  write(path.join(f.debug, "task", "trace.log"));
  const result = f.run(["--dry-run"]);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { directory: ".agent-debug/", entries: ["task"], dryRun: true });
  assert.equal(fs.readFileSync(path.join(f.debug, "task", "trace.log"), "utf8"), "keep");
});

test("dry-run and help do not create a missing directory", t => {
  const f = fixture(t);
  assert.equal(f.run(["--dry-run"]).status, 0);
  assert.equal(f.run(["--help"]).status, 0);
  assert.equal(fs.existsSync(f.debug), false);
});

test("a root directory link or Windows junction is refused without touching its target", t => {
  const f = fixture(t);
  const outside = path.join(f.directory, "outside");
  write(path.join(outside, "sentinel"));
  fs.symlinkSync(outside, f.debug, process.platform === "win32" ? "junction" : "dir");
  const result = f.run();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /symbolic link, or junction/);
  assert.equal(fs.readFileSync(path.join(outside, "sentinel"), "utf8"), "keep");
  assert.equal(fs.lstatSync(f.debug).isSymbolicLink(), true);
});

test("nested directory links are removed without following them outside the debug directory", t => {
  const f = fixture(t);
  const outside = path.join(f.directory, "outside");
  write(path.join(outside, "sentinel"));
  fs.mkdirSync(path.join(f.debug, "nested"), { recursive: true });
  fs.symlinkSync(outside, path.join(f.debug, "nested", "link"), process.platform === "win32" ? "junction" : "dir");
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(fs.readdirSync(f.debug), []);
  assert.equal(fs.readFileSync(path.join(outside, "sentinel"), "utf8"), "keep");
});

test("a file occupying the debug directory is refused and preserved", t => {
  const f = fixture(t);
  write(f.debug);
  const result = f.run();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /real directory/);
  assert.equal(fs.readFileSync(f.debug, "utf8"), "keep");
});

test("path arguments are refused before any cleanup", t => {
  const f = fixture(t);
  write(path.join(f.debug, "trace.log"));
  const result = f.run(["--path", f.repository]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Usage:/);
  assert.equal(fs.readFileSync(path.join(f.debug, "trace.log"), "utf8"), "keep");
});
