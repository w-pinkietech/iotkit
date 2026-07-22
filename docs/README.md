# IoTKit documentation

Choose the complete current product documentation for your language:

- [English](okf/en/index.md)
- [Japanese](okf/ja/index.md)

The two trees have the same relative paths, translation keys, document types,
statuses, and revisions. `scripts/check-okf-docs.mjs` enforces that structure and
requires both translations to change together.

## Authority

The `docs/okf/` bundle is the current human-readable product corpus. A versioned
contract consists of its language-paired contract document, machine-readable schema
or exported wire types, shared fixtures, and conformance tests. None silently
overrides the others; disagreement is a contract defect.

The short top-level compatibility documents under `docs/` are pointers only and are
not authorities. `docs/redesign/` and `docs/superpowers/` preserve rationale and
historical process records. They do not override the current corpus.
