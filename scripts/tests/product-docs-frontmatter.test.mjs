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

test("rejects non-string IoTKit fields instead of coercing YAML scalars", () => {
  const content = `---
type: 42
title: true
description: 123
language: en
translation_key: contracts.example
status: stable
revision: 1
---

body
`;
  const result = parseFrontmatterContent(content);
  assert.equal(result.error, undefined);
  const errors = validateRequiredScalars(result.metadata);
  assert.ok(errors.some((e) => e.includes("type must be a scalar string")));
  assert.ok(errors.some((e) => e.includes("title must be a scalar string")));
  assert.ok(errors.some((e) => e.includes("description must be a scalar string")));
});

test("preserves integer revision identity and rejects float or exponent notation", () => {
  const exact = parseFrontmatterContent(`---\n${baseScalars.replace("revision: 1", "revision: 9007199254740993")}\n---\n`);
  assert.equal(exact.error, undefined);
  assert.equal(exact.metadata.revision, "9007199254740993");
  assert.deepEqual(validateRequiredScalars(exact.metadata), []);

  for (const revision of ["1.0", "1e3"]) {
    const parsed = parseFrontmatterContent(`---\n${baseScalars.replace("revision: 1", `revision: ${revision}`)}\n---\n`);
    assert.equal(parsed.error, undefined);
    assert.ok(validateRequiredScalars(parsed.metadata).some((error) => error.includes("revision")), revision);
  }
});
