import { defineConfig } from "vite";

export default defineConfig({
  resolve: { dedupe: ["@xterm/xterm", "@xterm/addon-fit", "@xterm/addon-search"] },
  build: {
    outDir: "../../dist/ssh/frontend",
    emptyOutDir: true,
    lib: { entry: "src/index.ts", formats: ["es"], fileName: () => "index.js" },
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
