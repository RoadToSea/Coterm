#!/usr/bin/env node
/**
 * 模块化测试调度器 (Modular Test Runner)
 *
 * 支持按业务模块、测试类型或具体文件精准执行测试，避免每次全量运行 33 个用例。
 *
 * 用法：
 *   node scripts/test-runner.mjs                 # 运行全部测试
 *   node scripts/test-runner.mjs unit            # 仅运行纯内存单元测试 (< 1 秒)
 *   node scripts/test-runner.mjs browser         # 仅运行浏览器/UI 回归测试
 *   node scripts/test-runner.mjs serial          # 仅运行串口驱动相关测试
 *   node scripts/test-runner.mjs ssh             # 仅运行 SSH 驱动相关测试
 *   node scripts/test-runner.mjs desktop         # 仅运行桌面工作区/设置/审批测试
 *   node scripts/test-runner.mjs workspace       # 仅运行工作区与标签页测试
 *   node scripts/test-runner.mjs shared          # 仅运行共享 UI 与设计规范测试
 *   node scripts/test-runner.mjs <路径或关键字>   # 运行指定文件或匹配的目录
 */
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testsDir = path.join(rootDir, "tests");

function collectTestFiles(dir) {
  const files = [];
  if (!fs.existsSync(dir)) return files;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "support" && entry.name !== "fixtures" && entry.name !== "artifacts") {
        files.push(...collectTestFiles(full));
      }
    } else if (entry.name.endsWith(".test.mjs")) {
      files.push(full);
    }
  }
  return files;
}

const allTestFiles = collectTestFiles(testsDir);

const ALIAS_MAP = {
  drivers: ["tests/drivers"],
  serial: ["tests/drivers/serial"],
  ssh: ["tests/drivers/ssh"],
  local: ["tests/drivers/local"],
  network: ["tests/drivers/network"],
  tcp: ["tests/drivers/network"],
  telnet: ["tests/drivers/network"],

  desktop: ["tests/desktop"],
  workspace: ["tests/desktop/workspace"],
  approval: ["tests/desktop/approval"],
  settings: ["tests/desktop/settings"],

  shared: ["tests/shared"],
  ui: ["tests/shared/ui"],
  tokens: ["tests/shared/tokens"],

  system: ["tests/system"],
};

function isBrowserTest(filePath) {
  const name = path.basename(filePath);
  return name.includes("browser") || name.includes("host-interactions-component");
}

function resolveTargets(args) {
  if (args.length === 0 || args.includes("all")) {
    return allTestFiles;
  }

  if (args.includes("unit")) {
    return allTestFiles.filter((f) => !isBrowserTest(f));
  }

  if (args.includes("browser")) {
    return allTestFiles.filter((f) => isBrowserTest(f));
  }

  const selected = new Set();
  for (const arg of args) {
    const normalized = arg.replace(/\\/g, "/");
    if (ALIAS_MAP[normalized]) {
      for (const rel of ALIAS_MAP[normalized]) {
        const full = path.join(rootDir, rel);
        for (const file of collectTestFiles(full)) selected.add(file);
      }
      continue;
    }

    // Direct path check
    const directPath = path.resolve(rootDir, arg);
    if (fs.existsSync(directPath)) {
      if (fs.statSync(directPath).isDirectory()) {
        for (const file of collectTestFiles(directPath)) selected.add(file);
      } else {
        selected.add(directPath);
      }
      continue;
    }

    // Keyword match across all test files
    const matches = allTestFiles.filter((file) => {
      const rel = path.relative(rootDir, file).replace(/\\/g, "/");
      return rel.toLowerCase().includes(normalized.toLowerCase());
    });
    if (matches.length > 0) {
      for (const m of matches) selected.add(m);
    } else {
      console.warn(`[test-runner] Warning: No test files matched "${arg}"`);
    }
  }

  return Array.from(selected);
}

const args = process.argv.slice(2);
const targets = resolveTargets(args);

if (targets.length === 0) {
  console.error("[test-runner] Error: No test files found to run.");
  process.exit(1);
}

// Automatically ensure shared browser module and host interaction artifacts exist if running browser tests
const needsBrowserBuild = targets.some(isBrowserTest);
if (needsBrowserBuild) {
  const sharedModule = path.join(rootDir, "apps/shared/dist/browser/block-list.js");
  if (!fs.existsSync(sharedModule)) {
    console.log("[test-runner] apps/shared/dist/browser/block-list.js is missing, building...");
    const { execSync } = await import("node:child_process");
    execSync("node scripts/build-shared-browser.mjs", { cwd: rootDir, stdio: "inherit" });
  }

  const hostEntry = path.join(rootDir, "apps/desktop/dist/host-interactions/entry.js");
  if (!fs.existsSync(hostEntry)) {
    console.log("[test-runner] apps/desktop/dist/host-interactions/entry.js is missing, building...");
    const { execSync } = await import("node:child_process");
    execSync("node scripts/build-host-interactions-entry.mjs", { cwd: rootDir, stdio: "inherit" });
  }
}

const relTargets = targets.map((t) => path.relative(rootDir, t));
console.log(`[test-runner] Running ${relTargets.length} test file(s):`);
for (const t of relTargets) {
  console.log(`  - ${t}`);
}

const child = spawn(
  process.execPath,
  ["--import", "./scripts/test-ts-loader.mjs", "--test", ...relTargets],
  {
    cwd: rootDir,
    stdio: "inherit",
  }
);

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});
