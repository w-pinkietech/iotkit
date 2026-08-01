#!/usr/bin/env node
// Compatibility entry point. Prefer scripts/check-product-docs.mjs.
// Validates the product documentation tree packaged as an OKF producer profile.
import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const result = spawnSync(process.execPath, [path.join(dir, "check-product-docs.mjs"), ...process.argv.slice(2)], {
  stdio: "inherit",
  env: process.env,
});
process.exit(result.status ?? 1);
