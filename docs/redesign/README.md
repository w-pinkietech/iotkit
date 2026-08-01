# Redesign decisions and archive

This directory preserves the reasoning and evidence used while rewriting IoTKit.
It is **not** current product authority. Start at [`docs/README.md`](../README.md)
and [`docs/product/`](../product/).

Policy (see [#145](https://github.com/w-pinkietech/iotkit/issues/145)):

- Prefer absorbing still-true gaps into **`docs/product/`** over treating this tree as
  a living second corpus.
- Working gap survey: [`okf-gap-inventory.md`](okf-gap-inventory.md) ([#141](https://github.com/w-pinkietech/iotkit/issues/141)).
- `docs/superpowers/` is separate sprint history and is kept for lineage; it is
  not the primary absorb target.

### What is here

- `terminology.md`, `responsibility-ledger.md`, and `decisions/` — decision
  rationale. Implementation-status, wave, and roadmap lines are **as-of their
  write date**, not current status.
- `inputs/`, `reviews/`, `adr-inventory.md`, `rewrite-prep.md`, `diagrams/` —
  dated evidence. They often **do not match** today’s product; that is expected.
  Do not rewrite them to “fix” current organization.
- Surface text may still say Go Edge, Wave 0, host-agent, or obsolete authority
  rules. Trust **decision cores** only after checking product docs and code.

`docs/product/` is the current human-readable product corpus. Each versioned contract
consists of its language-paired contract document, machine-readable schema or
exported wire types, shared fixtures, and conformance tests. If this directory
conflicts with that set, stop and resolve intent; do not silently follow redesign.
Distill useful rationale into product docs (both languages, revision++); do not restore
historical implementation state as a requirement.
