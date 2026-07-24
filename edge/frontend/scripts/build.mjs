import { build } from "esbuild";

await build({
  bundle: true,
  entryPoints: [new URL("../src/console.ts", import.meta.url).pathname],
  format: "iife",
  legalComments: "none",
  outfile: new URL(
    "../static/console.js",
    import.meta.url,
  ).pathname,
  target: ["es2022"],
});
