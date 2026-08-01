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

### Product-docs freshness soft gate (CI)

On pull requests, lightweight CI also runs:

```bash
node scripts/product-docs-impact.mjs soft-check --base <base-sha> --pr-body-env PR_BODY
```

- **Warns** when impact candidates exist **and** the change has neither
  `docs/product/**` markdown updates **nor** a filled
  “No product-docs update reason / 更新しない理由” in the PR body.
- **Does not fail** the job and is **not a merge blocker**. Treat the warning as
  a prompt to update product docs or record a concrete no-update reason.
- Empty impact is still not a safety proof (same as the selector).
- Hard fail for high-risk paths is out of scope here (later issue).

## Supporting and historical trees

Neither development-process artifacts nor historical evidence override current
product docs.

| Tree | Role |
|---|---|
| `docs/product/` | Current product authority, packaged as OKF v0.2 |
| `docs/okf/` | Compatibility stub pointing to `docs/product/` |
| `docs/redesign/` | Early rewrite decisions and evidence; do not rewrite dated evidence to match today |
| `docs/superpowers/` | Optional issue-linked specifications and plans while work is active; frozen lineage after merge; never current product authority |

Create a Superpowers artifact only under the need-based rules in
[`workflow.md`](workflow.md). Absorb still-true historical gaps into product docs
using current terms; do not rewrite frozen evidence to make it look current.

Old IoTKit code is not an authority. If a task conflicts with a current
contract, stop and report the conflict.

Return to [`AGENTS.md`](../AGENTS.md).
