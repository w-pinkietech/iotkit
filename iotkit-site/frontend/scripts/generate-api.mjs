import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import openapiTS, { astToString } from "openapi-typescript";

const frontendRoot = fileURLToPath(new URL("../", import.meta.url));
const schema = new URL("../../openapi/site-console-v1.yaml", import.meta.url);
const output = fileURLToPath(
  new URL("../src/generated/site-api.d.ts", import.meta.url),
);

const nodes = await openapiTS(schema);
await mkdir(new URL("../src/generated/", import.meta.url), { recursive: true });
await writeFile(
  output,
  `// Generated from iotkit-site/openapi/site-console-v1.yaml. Do not edit.\n${astToString(nodes)}`,
);

console.log(`generated ${output.slice(frontendRoot.length)}`);
