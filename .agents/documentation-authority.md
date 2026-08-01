# Documentation authority

Start at [`docs/README.md`](../docs/README.md). The current human-readable
product documentation lives under `docs/product/`. Choose either `ja` or `en`
to read; edit both language files together.

The product tree is packaged as an OKF v0.2 bundle; see
[`docs/product/index.md`](../docs/product/index.md). Keep these layers distinct:

- **Authority:** `docs/product/`
- **Format:** OKF v0.2 packaging
- **Gate:** the IoTKit producer profile in `scripts/check-product-docs.mjs`

Do not treat “OKF” as a second corpus or as the name of the CI gate.
`docs/okf/` is a compatibility stub. A versioned contract is one artifact made
from its paired product documents, machine-readable schema or exported wire
types, shared fixtures, and conformance tests. None silently overrides the
others.

## Keep product docs current

`docs/product/` must describe the product as it is after a change merges.

- Write lasting product facts into `docs/product/` in the same issue and PR as
  the behavior or contract change.
- Keep investigation notes, discarded options, and one-off steps on the issue
  or PR.
- When a product concept changes, edit both `ja` and `en`, bump their shared
  `revision`, and run `node scripts/check-product-docs.mjs` (or the compatibility
  entry point `node scripts/check-okf-docs.mjs`).
- Record updated product-doc paths in the PR, or give a concrete reason why no
  product-doc update is needed.
- Optional OKF provenance fields (`sources`, `generated`, `verified`) are
  documented in `docs/product/<lang>/operations/okf-optional-meta.md`; they are
  not required by the product gate.

### Product-docs impact selector

Before opening or updating a PR, run a lower-bound path selector that maps
changed paths to candidate `docs/product/` files:

```bash
node scripts/product-docs-impact.mjs select --base origin/master
```

- Rules live in `scripts/docs/product-docs-impact-rules.json` (shared later with
  the CI soft gate in issue #165).
- Output lists candidate bilingual paths plus the matched rule and source paths.
- **Empty selection is not proof that product docs need no update.** Semantic or
  operator-visible changes still need a human judgment: update the corpus or
  record a concrete no-update reason on the PR.
- After editing the corpus, run `node scripts/check-product-docs.mjs` (form /
  IoTKit product gate). Impact answers “which docs might need a touch”; the
  checker answers “are the touched docs well-formed.”

## Historical trees

Neither historical tree overrides current product docs.

| Tree | Role |
|---|---|
| `docs/product/` | Current product authority, packaged as OKF v0.2 |
| `docs/okf/` | Compatibility stub pointing to `docs/product/` |
| `docs/redesign/` | Early rewrite decisions and evidence; do not rewrite dated evidence to match today |
| `docs/superpowers/` | Sprint designs and plans kept for lineage; do not add new specs or plans by default |

Absorb still-true historical gaps into product docs using current terms; do not
rewrite historical evidence to make it look current.

Old IoTKit code is not an authority. If a task conflicts with a current
contract, stop and report the conflict.

Return to [`AGENTS.md`](../AGENTS.md).
