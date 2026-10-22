/**
 * 让 node --test 直接 import 仓库里的 .ts 源码，不需要先构建，也不新增依赖。
 *
 * - 解析：为省略扩展名的相对导入补 .ts/.tsx/index，并把 `@coterm/plugin-sdk` 映射到 workspace 源码。
 * - 加载：交给 node:module 的类型剥离（node 22.13+ 自带），只去掉类型，不改语义。
 *
 * .tsx 只做类型剥离，不转换 JSX —— 组件渲染行为在有真实浏览器的用例里验证。
 */
import { registerHooks, stripTypeScriptTypes } from "node:module";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sdkRoot = path.join(repoRoot, "packages/plugin-sdk/src");
const SUFFIXES = ["", ".ts", ".tsx", "/index.ts", "/index.tsx"];

function resolveFile(specifier, parentURL) {
  let base;
  if (specifier.startsWith("@coterm/plugin-sdk")) {
    base = path.join(sdkRoot, specifier.slice("@coterm/plugin-sdk".length).replace(/^\//, ""));
  } else if (specifier.startsWith(".")) {
    base = path.resolve(path.dirname(fileURLToPath(parentURL)), specifier);
  } else {
    return null;
  }
  for (const suffix of SUFFIXES) {
    const candidate = `${base}${suffix}`;
    if (path.extname(candidate) !== "" && existsSync(candidate)) return candidate;
  }
  return null;
}

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (context.parentURL?.startsWith("file:")) {
      const resolved = resolveFile(specifier, context.parentURL);
      if (resolved) return { url: pathToFileURL(resolved).href, shortCircuit: true };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (!url.startsWith("file:") || !/\.tsx?$/.test(url)) return nextLoad(url, context);
    const source = readFileSync(fileURLToPath(url), "utf8");
    return {
      format: "module",
      source: stripTypeScriptTypes(source, { mode: "strip", sourceMap: false }),
      shortCircuit: true,
    };
  },
});
