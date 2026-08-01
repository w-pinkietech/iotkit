# Documentation authority

Start at [`docs/README.md`](../docs/README.md). The current human-readable product
corpus is `docs/okf/`. Choose either `ja` or `en` to read; edit both language
files together.

A versioned contract is one artifact made from its paired contract documents,
machine-readable schema or exported wire types, shared fixtures, and conformance
tests. None silently overrides the others.

## Historical trees

Neither overrides OKF. Policy detail: GitHub [#145](https://github.com/w-pinkietech/iotkit/issues/145).

| Tree | Role |
|---|---|
| `docs/okf/` | **Current** product corpus |
| `docs/redesign/` | Early rewrite decisions and evidence. Easy to misread as current law. Prefer absorbing still-true gaps into OKF. Do not rewrite dated evidence to “match today.” |
| `docs/superpowers/` | Sprint design/plans **kept for lineage**. Not current law. Do not add new specs/plans by default. Writing *style* may be reused on change lanes. |

Old IoTKit code is not an authority. If the task conflicts with a current
contract, stop and report the conflict.

Return to [`AGENTS.md`](../AGENTS.md).
