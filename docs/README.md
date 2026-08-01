# IoTKit documentation

Choose the complete current product documentation for your language:

- [English](product/en/index.md)
- [Japanese](product/ja/index.md)

For an Edge Node host failure or hardware replacement, start with the field
decision guide:

- [English recovery quick guide](product/en/operations/edge-node-hardware-recovery.md)
- [Japanese recovery quick guide](product/ja/operations/edge-node-hardware-recovery.md)

To change the product, start with the contributor guide:

- [English](../CONTRIBUTING.md)
- [Japanese](../CONTRIBUTING.ja.md)

The two language trees have the same relative paths, translation keys, document
types, statuses, and revisions. `scripts/check-product-docs.mjs` enforces that
structure and requires both translations to change together.

## Authority

The **`docs/product/`** tree is the current human-readable product corpus.
It is packaged as an [OKF v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundle: OKF is the format; product docs are the authority. A versioned contract
consists of its language-paired product document, machine-readable schema or
exported wire types, shared fixtures, and conformance tests. None silently
overrides the others; disagreement is a contract defect.

Keep the corpus current in the same change that alters lasting product facts.
Temporary notes stay on the issue or pull request. Process details:
[`AGENTS.md`](../AGENTS.md) (**Keep product docs current** and the change-lane table).

The short top-level compatibility documents under `docs/` are pointers only and are
not authorities. `docs/okf/` is a one-hop stub to `docs/product/` for old links.

Historical trees (neither overrides product docs; see
[#145](https://github.com/w-pinkietech/iotkit/issues/145)):

| Tree | Role |
|---|---|
| [`product/`](product/) | **Current** product corpus (OKF v0.2 packaging) |
| [`okf/`](okf/) | Compatibility stub → `product/` |
| [`redesign/`](redesign/) | Early rewrite decisions and evidence. Easy to misread as current law. Prefer absorbing still-true gaps into product docs ([#141](https://github.com/w-pinkietech/iotkit/issues/141)). Do not “fix” dated evidence to match today. |
| [`superpowers/`](superpowers/) | Sprint design/plans **kept for lineage**. Not current law; do not add new specs/plans by default. Writing *style* may be reused on light change lanes. Spec→product docs is secondary ([#143](https://github.com/w-pinkietech/iotkit/issues/143)). |

Development process (lanes, issue/PR loop) lives in [`AGENTS.md`](../AGENTS.md), not in these trees.

The Edge Node encrypted backup and fenced-candidate recovery contract is paired
across both language trees:

- [English recovery contract](product/en/contracts/edge-node-recovery-v1.md)
- [Japanese recovery contract](product/ja/contracts/edge-node-recovery-v1.md)
