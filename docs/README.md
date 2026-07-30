# IoTKit documentation

Choose the complete current product documentation for your language:

- [English](okf/en/index.md)
- [Japanese](okf/ja/index.md)

For an Edge Node host failure or hardware replacement, start with the field
decision guide:

- [English recovery quick guide](okf/en/operations/edge-node-hardware-recovery.md)
- [Japanese recovery quick guide](okf/ja/operations/edge-node-hardware-recovery.md)

To change the product, start with the contributor guide:

- [English](../CONTRIBUTING.md)
- [Japanese](../CONTRIBUTING.ja.md)

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

The Edge Node encrypted backup and fenced-candidate recovery contract is paired
across both language trees:

- [English recovery contract](okf/en/contracts/edge-node-recovery-v1.md)
- [Japanese recovery contract](okf/ja/contracts/edge-node-recovery-v1.md)
