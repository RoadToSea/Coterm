/**
 * 把 `packages/plugin-sdk/src/components/block-list.ts` 变成浏览器能直接 import 的 ES 模块。
 *
 * 只做 Node 自带的类型剥离，不打包、不复制实现：
 * 页面里跑的就是同一个源文件，因此 token 继承、选择状态与销毁行为都是真实组件的行为。
 * 产物进 dist/（已 gitignore），缺失时浏览器用例直接失败而不是回退替身。
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sourcePath = path.join(rootDir, "packages/plugin-sdk/src/components/block-list.ts");
const outDir = path.join(rootDir, "packages/plugin-sdk/dist/browser");
const outPath = path.join(outDir, "block-list.js");

const source = readFileSync(sourcePath, "utf8");
const stripped = stripTypeScriptTypes(source, { mode: "strip", sourceMap: false });
// 类型剥离会留下纯类型 import；浏览器无法解析，去掉后模块只剩运行时代码。
const runtime = stripped.replace(/^\s*import\s+type\s[^;]*;\s*$/gm, "");

mkdirSync(outDir, { recursive: true });
writeFileSync(outPath, runtime, "utf8");
console.log(`built ${path.relative(rootDir, outPath)} from ${path.relative(rootDir, sourcePath)}`);
