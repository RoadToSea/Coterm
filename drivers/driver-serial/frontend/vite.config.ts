import { defineConfig } from "vite";

export default defineConfig({
  build: {
    outDir: "../dist/frontend",
    emptyOutDir: true,
    lib: { entry: "src/index.ts", formats: ["es"], fileName: () => "index.js" },
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
