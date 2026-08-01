# Historical design and implementation records

The `specs/` and `plans/` directories preserve completed development-process
artifacts. They may contain old names (including former Site vocabulary),
superseded scope, unchecked steps, or implementation state from a specific
point in time.

These files are not current work instructions and do not override code, executable
contracts, current contract documents, or [`docs/README.md`](../README.md). New work
should update the current source that owns the behavior instead of treating an old
plan as a living specification.

## Keep this tree (lineage)

Policy ([#145](https://github.com/w-pinkietech/iotkit/issues/145)):

- **Keep this directory permanently for lineage.** Do not bulk-delete, rename,
  or move it to an archive.
- **Do not add** new specs or plans for ongoing work by default.
- Reuse the **writing style** (goal, non-goals, decision, verification; optional
  plan for Full work) on AGENTS change lanes—not by growing this tree.
- Specs→OKF absorption is **secondary**. Primary product-gap work is redesign→OKF
  ([#141](https://github.com/w-pinkietech/iotkit/issues/141)). Spec gap survey:
  [`specs-okf-gap-inventory.md`](specs-okf-gap-inventory.md)
  ([#143](https://github.com/w-pinkietech/iotkit/issues/143)).
- `plans/` are execution logs only—never normative.

Current process: Change lanes in [`AGENTS.md`](../../AGENTS.md) (Fast default).
