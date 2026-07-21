import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import openapiTS, { astToString } from "openapi-typescript";

const temporary = await mkdtemp(join(tmpdir(), "iotkit-edge-console-"));

try {
  const schema = new URL("../../openapi/edge-console-v1.yaml", import.meta.url);
  const expectedTypes = fileURLToPath(
    new URL("../src/generated/edge-api.d.ts", import.meta.url),
  );
  const generatedTypes = join(temporary, "edge-api.d.ts");
  const nodes = await openapiTS(schema);
  const types =
    `// Generated from iotkit-edge/openapi/edge-console-v1.yaml. Do not edit.\n` +
    astToString(nodes);
  await import("node:fs/promises").then(({ writeFile }) =>
    writeFile(generatedTypes, types),
  );

  const expectedBundle = fileURLToPath(
    new URL("../../internal/edgehttp/static/console.js", import.meta.url),
  );
  const generatedBundle = join(temporary, "console.js");
  await build({
    bundle: true,
    entryPoints: [new URL("../src/console.ts", import.meta.url).pathname],
    format: "iife",
    legalComments: "none",
    outfile: generatedBundle,
    target: ["es2022"],
  });

  const comparisons = [
    ["OpenAPI types", expectedTypes, generatedTypes],
    ["Console JavaScript", expectedBundle, generatedBundle],
  ];
  for (const [label, expected, generated] of comparisons) {
    const expectedContent = await readFile(expected);
    const generatedContent = await readFile(generated);
    if (!expectedContent.equals(generatedContent)) {
      throw new Error(
        `${label} is stale. Run npm --prefix iotkit-edge/frontend run generate:api && npm --prefix iotkit-edge/frontend run build.`,
      );
    }
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}
