# Redesign decisions and archive

This directory preserves the reasoning and evidence used while rewriting IoTKit.
It is not the first source for current product behavior, but current documents may
still cite its decisions for rationale and invariants.

- `terminology.md`, `responsibility-ledger.md`, and `decisions/` are the decision
  corpus. They define still-cited terms, rationale, and invariants. Their
  implementation-status, wave, and roadmap statements describe the date on which
  they were written and are not current status authority.
- `inputs/`, `reviews/`, and `adr-inventory.md` are historical evidence and review
  provenance, not product requirements.
- Current authority and reading order are defined in [`docs/README.md`](../README.md).

Machine-readable contract artifacts and their current contract document form one
contract set. If this directory conflicts with that set or current architecture,
stop and resolve the design intent instead of silently changing one side. Preserve
useful rationale by gradually distilling it into a current concept, contract, or
short decision document; do not restore historical implementation state as a
requirement.
