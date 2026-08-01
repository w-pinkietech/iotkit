import assert from "node:assert/strict";
import test from "node:test";
import { parseFrontmatterContent, validateRequiredScalars } from "../docs/frontmatter.mjs";

const baseScalars = `
type: Contract
title: "Example"
description: "Nested OKF families are allowed."
language: en
translation_key: contracts.example
status: stable
revision: 1
`.trim();

test("parses nested OKF v0.2 families without rejecting them", () => {
  const content = `---
${baseScalars}
generated: { by: human:reviewer, at: 2026-08-01T12:00:00Z }
verified:
  - { by: human:reviewer, at: 2026-08-01T12:05:00Z }
sources:
  - id: schema
    resource: https://example.invalid/schema
    title: Example schema
stale_after: 2027-01-01
---

# Body
`;
  const result = parseFrontmatterContent(content);
  assert.equal(result.error, undefined);
  assert.equal(result.metadata.type, "Contract");
  assert.equal(result.metadata.revision, "1");
  assert.deepEqual(result.metadata.generated, {
    by: "human:reviewer",
    at: "2026-08-01T12:00:00Z",
  });
  assert.equal(result.metadata.sources[0].id, "schema");
  assert.equal(result.metadata.stale_after, "2027-01-01");
  assert.equal(validateRequiredScalars(result.metadata).length, 0);
});

test("rejects invalid YAML frontmatter", () => {
  const content = `---
type: Contract
title: [unterminated
---

body
`;
  const result = parseFrontmatterContent(content);
  assert.match(result.error ?? "", /invalid YAML frontmatter/);
});

test("rejects missing frontmatter", () => {
  const result = parseFrontmatterContent("# no frontmatter\n");
  assert.equal(result.error, "missing YAML frontmatter");
});

test("reports missing IoTKit required scalars", () => {
  const content = `---
type: Contract
title: "Only type and title"
---

body
`;
  const result = parseFrontmatterContent(content);
  assert.equal(result.error, undefined);
  const errors = validateRequiredScalars(result.metadata);
  assert.ok(errors.some((e) => e.includes("description")));
  assert.ok(errors.some((e) => e.includes("translation_key")));
});
