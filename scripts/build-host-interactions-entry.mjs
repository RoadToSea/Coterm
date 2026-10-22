/**
 * 用 Vite 把「只挂 HostInteractions」的测试入口打成页面模块。
 *
 * 不能直接跑真实 App：它会同时挂 HostApprovals，两者的快照记录会混在一起，
 * 反而看不出 HostInteractions 自己的弹窗生命周期。所以这里只 import 那一个组件，
 * 其余全部走真实路径（host-api、secret-input、React、Tauri API）。
 *
 * 产物与产品构建放在同一目录下（dist/host-interactions/），
 * `assertFreshBuild` 会检查它不比相关源码旧。
 */
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = path.join(rootDir, "apps/desktop");
const desktopRequire = createRequire(path.join(desktopDir, "package.json"));

/** 入口在 apps/desktop 之外，裸模块要按产品自己的 node_modules 解析。 */
function aliasFor(specifier) {
  try {
    return desktopRequire.resolve(specifier);
  } catch {
    return null;
  }
}

const BARE = [
  "react",
  "react-dom",
  "react-dom/client",
  "@tauri-apps/api/core",
  "@tauri-apps/api/event",
];

export async function buildHostInteractionsEntry() {
  const { build } = await desktopRequire("vite");
  // 精确匹配，避免 "react-dom" 前缀吃掉 "react-dom/client"。
  const bareAlias = BARE.map((specifier) => {
    const resolved = aliasFor(specifier);
    if (!resolved) throw new Error(`cannot resolve ${specifier} from apps/desktop`);
    return { find: new RegExp(`^${specifier.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`), replacement: resolved };
  });
  await build({
    root: desktopDir,
    configFile: false,
    logLevel: "warn",
    // 与产品构建一致：React 走 production 分支，浏览器里没有 process。
    define: { "process.env.NODE_ENV": '"production"' },
    resolve: {
      alias: [
        ...bareAlias,
        {
          find: /^@coterm\/plugin-sdk\/components\/block-list$/,
          replacement: path.join(rootDir, "packages/plugin-sdk/src/components/block-list.ts"),
        },
        {
          find: /^@coterm\/plugin-sdk$/,
          replacement: path.join(rootDir, "packages/plugin-sdk/src/index.ts"),
        },
      ],
    },
    build: {
      outDir: path.join(desktopDir, "dist/host-interactions"),
      emptyOutDir: true,
      lib: {
        entry: path.join(rootDir, "tests/support/host-interactions-entry.tsx"),
        formats: ["es"],
        fileName: () => "entry.js",
      },
      rollupOptions: { output: { inlineDynamicImports: true } },
    },
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await buildHostInteractionsEntry();
  console.log("built apps/desktop/dist/host-interactions/entry.js");
}
