# IoTKit documentation

Choose the complete current product documentation for your language:

- [English](product/en/index.md)
- [Japanese](product/ja/index.md)

For an Edge Node host failure or hardware replacement, start with the
installation and recovery runbook:

- [English](product/en/operations/installation-and-recovery.md)
- [Japanese](product/ja/operations/installation-and-recovery.md)

To change the product, start with the contributor guide:

- [English](../CONTRIBUTING.md)
- [Japanese](../CONTRIBUTING.ja.md)

The two language trees have the same relative paths, translation keys, document
types, statuses, and revisions. `scripts/check-product-docs.mjs` enforces that
structure and requires both translations to change together.

## Authority

The **`docs/product/`** tree is the current human-readable product corpus.
It is packaged as an [OKF v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundle: OKF is the **format**; product docs are the **authority**; CI runs the
**IoTKit producer profile** gate (stricter than plain OKF consumers—see
[product/index.md](product/index.md)). A versioned contract consists of its
language-paired product document, machine-readable schema or exported wire types,
shared fixtures, and conformance tests. None silently overrides the others;
disagreement is a contract defect.

Keep the corpus current in the same change that alters lasting product facts.
Temporary notes stay on the issue or pull request. Process details:
[`AGENTS.md`](../AGENTS.md) (**Keep product docs current** and the change-lane table).

The short top-level compatibility documents under `docs/` are pointers only and are
not authorities. `docs/okf/` is a one-hop stub to `docs/product/` for old links.

Documentation trees (supporting and historical rows never override product docs; see
[#145](https://github.com/w-pinkietech/iotkit/issues/145)):

| Tree | Role |
|---|---|
| [`product/`](product/) | **Current** product corpus (OKF v0.2 packaging) |
| [`okf/`](okf/) | Compatibility stub → `product/` |
| [`redesign/`](redesign/) | Early rewrite decisions and evidence. Easy to misread as current law. Prefer absorbing still-true gaps into product docs ([#141](https://github.com/w-pinkietech/iotkit/issues/141)). Do not “fix” dated evidence to match today. |
| [`superpowers/`](superpowers/) | Optional issue-linked design and implementation artifacts while work is active; frozen lineage after merge. Never current product law. Create only when the change-lane criteria need durable spec or plan context. |

Development process (lanes, issue/PR loop, and artifact creation criteria) lives
in [`AGENTS.md`](../AGENTS.md); `superpowers/` stores selected process artifacts
but does not define the process.
