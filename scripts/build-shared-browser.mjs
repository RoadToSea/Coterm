import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL("../", import.meta.url));
const desktop = path.join(root, "apps/desktop");
const require = createRequire(path.join(desktop, "package.json"));
const { build } = require("vite");
process.chdir(desktop);
await build({
  root: desktop, configFile: path.join(desktop, "vite.config.ts"),
  define: { "process.env.NODE_ENV": '"production"' },
  build: {
    outDir: path.join(desktop, "dist/shared-ui"), emptyOutDir: true,
    lib: { entry: path.join(root, "tests/support/shared-ui-entry.ts"), formats: ["es"], fileName: () => "index.js" },
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
