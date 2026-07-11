# Active Development Ledger

This is persistent workflow state, not design authority. Verify every factual claim against
git/disk/test. Never store secrets here.

## Reality

- Repository: `iotkit-next`
- Branch: `master`
- Artifact/base HEAD: `203ee4c`
- Working tree at workflow-design start: user-owned untracked
  `docs/eval/autonomous-development-policy-discussion-2026-07-11.md`
- Active phase: workflow acceleration adoption; Plan 6 product design frozen

## Mission

- Adopt risk-adaptive autonomous delivery with Design Ready, Green/Yellow/Red, three-vendor
  review, and persistent restart state.
- Preserve independent review, TDD, verification, design canon, and product-code worker
  separation.
- Trial Yellow autonomy on Plan 6.
- External push/PR/release is not authorized by this mission.

## User decisions

- 2026-07-11: Codex becomes Main driver; cross-vendor review remains mandatory.
- 2026-07-11: Normal review matrix is Codex high, Claude Fable/high, Grok high;
  high-risk review escalates to strongest pinned models/max.
- 2026-07-11: Add local Grok Build as the third review vendor.
- 2026-07-11: Improve the process to accelerate development; adopt the discussion input via
  reviewed workflow changes.
- Plan 6 product decision already approved: no network box claim; initial admin ownership is
  local/per-card only. This design is not SETTLED yet.

## Review state

- Workflow-policy review round 1: COMPLETE, NOT SETTLED. Prompt
  `/tmp/workflow-autonomy-review.md` SHA-256
  `6d3ad4ad7276591cfad8d71a311d354830d245cb9c6b3cdf22240eff1a0a2b0d`; legacy manifest
  Git-object hash `7ccb574d2942af1375e9b606e0a652239b872df8`. Results:
  `/tmp/codex-runs/codex-workflow-autonomy-review-20260711-131705-318411.txt`,
  `/tmp/codex-runs/claude-workflow-autonomy-review-20260711-131705-318412.txt`, and
  `/tmp/codex-runs/grok-workflow-autonomy-review-20260711-131705-318419.txt`.
  All three returned adopt-after-fixes; fixes are in progress. This round predates receipts
  and cannot establish final settlement.
- Workflow-policy final round: READY TO DISPATCH. Substantive manifest
  `.review/workflow-final.manifest` SHA-256
  `626b9b422795bed4a8eef9ac272efd3e4c4eab06c7691a5e18d7b403b0edcba0`; prompt
  `.review/workflow-final-review.md` SHA-256
  `5c23b7f56a156782d9134f580792ac556ffec7e28ea9feca830c8f2bc85f751b`.
  Vendors owed: Codex (`gpt-5.6-sol/max`), Claude (`opus/max`), Grok
  (`grok-4.5/max`). Expected result and receipt directory: `/tmp/codex-runs/`.
  Per-vendor state: Codex=PENDING, Claude=PENDING, Grok=PENDING.
- Workflow max discovery round completed with bound receipts. Codex and Grok returned
  adopt-after-fixes; Claude output exposed plan-mode external writing and its findings were
  recovered from the generated plan. Confirmation artifact is
  `.review/workflow-confirm.manifest` SHA-256
  `58a414c768644d54d540a1ba02debd8286a322875d57a577d6e3613dbb3a8cd7`; prompt SHA-256
  `fa7d55f6261c38b2edc54fa0131a4781daa906dda71c50346ade9df242d05789`.
  Confirmation vendors owed: Codex/Fable/Grok at high; all PENDING.
- Final zero-C/I round prepared: `.review/workflow-final2.manifest` SHA-256
  `697ddfb5b1ef810fe7e323a46b2979ef58e7dccfc091423ce4b2de89fc12cb27`; prompt SHA-256
  `4cb2a6b7edf60357cfdb2bddb251d33490a3e8491144c739846475ec89c3a021`.
  Vendors owed: Codex/Fable/Grok high; all PENDING.
- Final4 exact-mechanical confirmation: `.review/workflow-final4.manifest` SHA-256
  `ce1eb15a546ff4e89f7d54c2f3c80bf4e0957e466074306fcf70345291559beb`; prompt SHA-256
  `547dce1b091478b17dc3340640d6ade1cbd0458df4a1d0454f51fd774fd47d03`.
  **SETTLED** with zero unresolved C/I from Codex/Fable/Grok at high. Results and receipt
  SHA-256 values:
  - Codex result `5b98087798e080008fba9277b071e3dc7df1ad6adae369f9047acaedc04369ee`,
    receipt `0be22a38f391298df7d378e9d6ba7660e4da98c6a651aa778005015645033105`.
  - Claude result `f1d501371f31252010d7b6daec16c31b5e3c594f589cdc6f9b366a5ce796bc5b`,
    receipt `087a2a54101b298bc47aceb20b178b4b03692f4c5594e308b6442116936756e2`.
  - Grok result `8343067f05b8ca433b82832cb9ba4eea24269fff0216638eb34fb1323af48629`,
    receipt `5fa80d237409deb8229bd2f60c3ebcd831bbe0491194ac1549856b0fd1c92899`.
- Final3 artifact after Claude host-read closure: `.review/workflow-final3.manifest` SHA-256
  `f2f552e06ebe1c70849ffadd83f32e4928500c671cc102c294ccfa66068b0e15`; prompt SHA-256
  `831b9d62d2a2951e66bd74c292b1d2d0d1ce611b4aff77f1c2ce1339ceaa8a3b`.
  Vendors owed: Codex/Fable/Grok high; all PENDING.
- Plan-6 local-bootstrap revision 2: NOT SETTLED. All three reviews completed on
  `ae69304f68e3e566e8e58c569984ccfc4cf3920d`; Codex/Claude found unresolved restore,
  preseed-replay, and local-trust-boundary issues. Product work remains frozen until resumed
  under this workflow.

## Unresolved Red packet

Prepare one bundled Plan-6 packet after the workflow is SETTLED. Candidate items:

1. R22 restore versus auth revocation-generation rollback.
2. Factory reset meaning and authority.
3. Product scope/UX consequence of the local-only bootstrap producer and recovery limit.

Do not ask these separately. Continue Design Ready evidence that is common to all choices.

## Yellow decisions

- Canonical workflow location: `docs/development-workflow.md`; reversible by reverting the
  workflow commit. Reconsider if role-specific docs drift or agents fail to load it.
- Persistent state location: this file. It is deliberately Git-tracked and non-canonical for
  product design. Reconsider if update churn obscures product commits.
- Current cumulative roll-up: these workflow-location choices remain reversible and do not
  alter the approved product envelope. Reviewers must confirm this classification on the final
  workflow round.

## Next executable work

1. Commit the SETTLED workflow/harness artifact.
2. Resume Plan 6 with its Design Ready pack and one bundled Red packet.

## Verification

- Round-1 pre-dispatch: `bash -n`, `git diff --check`, watchpoints, and Grok low smoke passed.
- Final: wrapper negative probes, tool-free Fable smoke, isolated Grok smoke, mode-aware
  manifest, bound receipts, `bash -n`, `git diff --check`, watchpoints, and Final4
  three-vendor zero-C/I confirmation passed.
