import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const desktop = path.join(root, "apps/desktop");
const require = createRequire(path.join(desktop, "package.json"));
const { build } = await require("vite");
process.chdir(desktop);
for (const kind of ["local", "ssh", "serial", "tcp", "telnet"]) {
  await build({
    root: desktop,
    configFile: path.join(desktop, "vite.config.ts"),
    logLevel: "warn",
    define: { __WORKBENCH_KIND__: JSON.stringify(kind), "process.env.NODE_ENV": '"production"' },
    build: {
      outDir: path.join(desktop, "dist/workbenches", kind),
      emptyOutDir: true,
      lib: { entry: path.join(root, "tests/support/workbench-entry.tsx"), formats: ["es"], fileName: () => "index.js" },
      rollupOptions: { output: { inlineDynamicImports: true } },
    },
  });
}
