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
not authorities.

Historical trees (neither overrides OKF; see
[#145](https://github.com/w-pinkietech/iotkit/issues/145)):

| Tree | Role |
|---|---|
| [`okf/`](okf/) | **Current** product corpus |
| [`redesign/`](redesign/) | Early rewrite decisions and evidence. Easy to misread as current law. Prefer absorbing still-true gaps into OKF ([#141](https://github.com/w-pinkietech/iotkit/issues/141)). Do not “fix” dated evidence to match today. |
| [`superpowers/`](superpowers/) | Sprint design/plans **kept for lineage**. Not current law; do not add new specs/plans by default. Writing *style* may be reused on light change lanes. Spec→OKF is secondary ([#143](https://github.com/w-pinkietech/iotkit/issues/143)). |

Development process (lanes, issue/PR loop) lives in [`AGENTS.md`](../AGENTS.md), not in these trees.

The Edge Node encrypted backup and fenced-candidate recovery contract is paired
across both language trees:

- [English recovery contract](okf/en/contracts/edge-node-recovery-v1.md)
- [Japanese recovery contract](okf/ja/contracts/edge-node-recovery-v1.md)
