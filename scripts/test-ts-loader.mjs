/**
 * 让 node --test 直接 import 仓库里的 .ts 源码，不需要先构建，也不新增依赖。
 *
 * - 解析：为省略扩展名的相对导入补 .ts/.tsx/index。
 * - 加载：.ts 使用 node:module 类型剥离，.tsx 使用项目已有的 TypeScript 转换 JSX。
 *
 * 组件的实际渲染与交互仍由真实浏览器用例验证。
 */
import { createRequire, registerHooks, stripTypeScriptTypes } from "node:module";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const SUFFIXES = ["", ".ts", ".tsx", "/index.ts", "/index.tsx"];
const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const require = createRequire(path.join(desktop, "package.json"));

function resolveFile(specifier, parentURL) {
  if (!specifier.startsWith(".")) return null;
  const base = path.resolve(path.dirname(fileURLToPath(parentURL)), specifier);
  for (const suffix of SUFFIXES) {
    const candidate = `${base}${suffix}`;
    if (path.extname(candidate) !== "" && existsSync(candidate)) return candidate;
  }
  return null;
}

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (context.parentURL?.startsWith("file:")) {
      const resolved = specifier.startsWith("@/") ? resolveFile(`./${specifier.slice(2)}`, pathToFileURL(path.join(desktop, "src/index.ts")).href) : resolveFile(specifier, context.parentURL);
      if (resolved) return { url: pathToFileURL(resolved).href, shortCircuit: true };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (!url.startsWith("file:") || !/\.tsx?$/.test(url)) return nextLoad(url, context);
    const source = readFileSync(fileURLToPath(url), "utf8");
    return {
      format: "module",
      source: url.endsWith(".tsx") ? require("typescript").transpileModule(source, {
        compilerOptions: { jsx: require("typescript").JsxEmit.ReactJSX, module: require("typescript").ModuleKind.ESNext, target: require("typescript").ScriptTarget.ES2020 },
      }).outputText : stripTypeScriptTypes(source, { mode: "strip", sourceMap: false }),
      shortCircuit: true,
    };
  },
});
