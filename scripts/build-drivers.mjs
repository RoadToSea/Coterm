import { execFileSync } from "node:child_process";
import { mkdirSync, copyFileSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const pnpm = process.env.npm_execpath;
if (!pnpm) throw new Error("Run with pnpm drivers:build");
const release = process.argv.includes("--release");
const destination = path.resolve(root, "target/bundled-drivers");
for (const name of ["ssh", "serial", "local"]) {
  execFileSync(process.execPath, [pnpm, `driver:${name}:build`, ...(release ? ["--release"] : [])], {cwd:root,stdio:"inherit"});
  const source = path.join(root, `drivers/driver-${name}/dist`);
  const manifest = JSON.parse(readFileSync(path.join(source,"manifest.json"),"utf8"));
  if (manifest.id !== `coterm-driver-${name}`) throw new Error(`Unexpected ${name} driver identity`);
  const target = path.resolve(destination,name);
  if (path.dirname(target) !== destination) throw new Error("Invalid driver staging destination");
  rmSync(target,{recursive:true,force:true});
  mkdirSync(path.join(target,"backend"),{recursive:true});
  mkdirSync(path.join(target,"frontend"),{recursive:true});
  copyFileSync(path.join(source,"manifest.json"),path.join(target,"manifest.json"));
  copyFileSync(path.join(source,"frontend/index.js"),path.join(target,"frontend/index.js"));
  for (const entry of Object.values(manifest.backend)) copyFileSync(path.join(source,entry),path.join(target,entry));
}
