# Active Development Ledger

This is persistent workflow state, not design authority. Verify every factual claim against
git/disk/test. Never store secrets here.

## Reality

- Repository: `iotkit-next`
- Branch: `master`
- Artifact/base HEAD: `8a4245b`
- Working tree at workflow-design start: user-owned untracked
  `docs/eval/autonomous-development-policy-discussion-2026-07-11.md`
- Active phase: workflow acceleration adoption; Plan 6 product design frozen

## Mission

- Adopt risk-adaptive autonomous delivery with Design Ready, Green/Yellow/Red, independent
  review, and persistent restart state.
- Preserve independent review, TDD, verification, design canon, and product-code worker
  separation.
- Trial Yellow autonomy on Plan 6.
- External push/PR/release is not authorized by this mission.

## User decisions

- 2026-07-11: Codex becomes Main driver; cross-vendor review remains mandatory (superseded
  2026-07-12 while both external vendors are unavailable).
- 2026-07-11: Normal review matrix was Codex high, Claude Fable/high, Grok high;
  high-risk review escalated to strongest pinned models/max. **Superseded 2026-07-12** by the
  official-use routing below; do not use this historical matrix for new dispatches.
- 2026-07-11: Add local Grok Build as the third review vendor.
- 2026-07-12: Grok quota is exhausted. Required settlement vendors were Codex and Claude;
  Grok remains optional and creates no review debt unless explicitly opted in before dispatch.
- 2026-07-12: Claude subscription access was disabled and returns HTTP 403. Cross-vendor review
  is temporarily impossible. Required review is a fresh read-only Codex session; Claude/Grok
  are optional and create no debt unless explicitly opted in before dispatch.
- 2026-07-12: Adopt the official-use model/effort routing: Luna/low for clear repeatable
  mechanical work, Terra/medium for everyday settled-spec implementation, Sol/medium for
  normal independent review, and Sol/high for design and high-risk work. Plan 6 uses Sol/high;
  `xhigh` is exceptional and `max` is not a routine default.
- 2026-07-11: Improve the process to accelerate development; adopt the discussion input via
  reviewed workflow changes.
- Plan 6 product decision already approved: no network box claim; initial admin ownership is
  local/per-card only. This design is not SETTLED yet.

## Review state

- Model/effort routing policy: **SETTLED**. Round 1 substantive manifest
  `.review/model-routing.manifest` SHA-256
  `7a4dcefe1fae3943b62a85c07368d3edbf3c4f7218804613c4dfa9a840def13c`; prompt
  `.review/model-routing-review.md` round-1/2 SHA-256
  `25a39365cd824df0e47f29f4648045bd6aae96076e75f5515dbebaca0309d09a`.
  Fresh Codex `gpt-5.6-sol/high` found two Important legacy-routing contradictions; both were
  fixed. Result SHA-256
  `e6bd7b4a8433479af5a1cdaa0d3066fe3a448b88119a0e91f134e6ceabee9dfc`; receipt SHA-256
  `bcabc8d50ff2ed59e7d50764c5bac2a56e4e0fdd2bc9ab5f5cdb877b2af9a9a6`.
  Confirmation round 2 on `.review/model-routing-final.manifest` SHA-256
  `d7deb30528cc0f6216144d55dad05a446cff9595e6b4dc1fa0db39f4b6933884` found one Important
  stale eval-skill instruction, now fixed. Result SHA-256
  `6a023e33d9dd8d0bb2296f186846d808a56cd244b40c600a4e91d4993994f0b9`; receipt SHA-256
  `29f540c3743faa5f1aa8b0e078b6c2812da375912ad10eacc3f50927485c54f8`.
  Starting with round 3, the prompt added shared eval-skill consistency checking; its SHA-256 is
  `80b83ed6b696bbc93a344a971b9e9ced591064071bb6dd63beed0ee06413890f`.
  Confirmation round 3 on `.review/model-routing-final2.manifest` SHA-256
  `ea39c22f900fea8941e8cc60915bb0f352cdb9f0b3510b543624bdf313736dcd` found one Important
  stale cross-vendor requirement in the optional Claude wrapper header, now fixed. Result
  SHA-256 `0d32ad4e189dc5f61fe714c2cb0e74cfc91555af2e597e03c71a1beb3f4685b7`; receipt SHA-256
  `7119e993461844c9dd141845097fd72bccb4b3f808529372781de45803bbcf9f`.
  Confirmation round 4 on `.review/model-routing-final3.manifest` SHA-256
  `2686e57f956c37d1fcfd18b8af8bd9ce315e694ec8978fbca476735b14ebeebb` found one Important
  stale next-action ordering in this operational ledger, now fixed. Result SHA-256
  `0597022f20c82ff2ecf2bf5e80672fc4687bf63b904b824446c85b7c0abc05c9`; receipt SHA-256
  `2ce32e606b1edcb0c966179389dff1c69f1a62351286fa4724491127b0da6f8c`.
  Confirmation round 5 on the same manifest found one Important high-risk downgrade path;
  Plan 6 and every Large/Red or design workflow are now pinned to Sol/high through final
  settlement. Result SHA-256
  `ccca2914b6ce996d4feffa25a0a235a65f4fd526e7441b6b4324274cb4153e4e`; receipt SHA-256
  `a691dbe3e4b6dac1a2f11d133ca2373d69fe99d63f430a1315b604f5a6914a26`.
  Confirmation round 6 on `.review/model-routing-final5.manifest` found one Important stale
  prompt-provenance description in this operational ledger, now fixed. Result SHA-256
  `a4a31e7c7826e0b1a523e4416aed4532ae4efda1ee21edb344c2687a8cccc6e1`; receipt SHA-256
  `7397211c5262605f7eb12097e5e310e9e871105fda21c64396f5c92497ff36e7`.
  Final confirmation completed on `.review/model-routing-final5.manifest` SHA-256
  `dff5c9f32a19b5c3a1eca358b4c1c40e3caa141a21982eadb56fd503c5a437b8`; prompt
  `.review/model-routing-review.md` SHA-256
  `80b83ed6b696bbc93a344a971b9e9ced591064071bb6dd63beed0ee06413890f`.
  Fresh Codex `gpt-5.6-sol/high` returned zero unresolved C/I. Result SHA-256
  `13cc67240d0f977eb77aa73a82a7e3d291a6e245e95928b181886a6dde903cf6`; receipt SHA-256
  `3a134260335a2ef1ee07e8a75403d9bbb7dbd65be67b0e90e1f4a953e44e33fa`.
  Claude/Grok were unavailable and not required.

- Review-policy restoration: REVIEW IN FLIGHT. Manifest
  `.review/two-vendor-policy.manifest` SHA-256
  `204f56da63e074f6d33a3fadcdf01d91149084ccaa936a64684a802ca9e3efc5`; prompt SHA-256
  `f024b0b6525aa5ff4b0b80a3448785ea00f9e7e9495a30a774a0f18187de732d`.
  Codex medium completed with two Important findings; Claude failed with HTTP 403 and is no
  longer required under the superseding user decision. Grok=NOT REQUIRED.
- Degraded-review final: **SETTLED**. Manifest
  `.review/degraded-review-policy.manifest` SHA-256
  `886502e52cd0e6342d969dbbd8762b0e67b3bec6ebe4b332346f927457190d96`; prompt SHA-256
  `c37c633ed676403437a8071e139c6881d21ba08b431d398da69d3c8ca161cce9`.
  Fresh Codex medium returned zero C/I. Result SHA-256
  `e9510aa77ce3c33da9269017d8e4f1a5ae9cc2e55fdc3ed09fb54372374de9ac`; receipt SHA-256
  `bb7232fb391c98e6d7844caf341d3440ae7d291a2dd4b288d4b7dba0bd4883eb`.
  Claude/Grok=UNAVAILABLE, NOT REQUIRED.

- Workflow-policy review round 1: COMPLETE, NOT SETTLED. Prompt
  `/tmp/workflow-autonomy-review.md` SHA-256
  `6d3ad4ad7276591cfad8d71a311d354830d245cb9c6b3cdf22240eff1a0a2b0d`; legacy manifest
  Git-object hash `7ccb574d2942af1375e9b606e0a652239b872df8`. Results:
  `/tmp/codex-runs/codex-workflow-autonomy-review-20260711-131705-318411.txt`,
  `/tmp/codex-runs/claude-workflow-autonomy-review-20260711-131705-318412.txt`, and
  `/tmp/codex-runs/grok-workflow-autonomy-review-20260711-131705-318419.txt`.
  All three returned adopt-after-fixes; fixes are in progress. This round predates receipts
  and cannot establish final settlement.
- Workflow-policy final round: **HISTORICAL / CANCELLED; DO NOT DISPATCH**. This prepared
  max-effort three-vendor round was superseded by the later completed Final4 settlement and
  the 2026-07-12 degraded-review policy. Substantive manifest
  `.review/workflow-final.manifest` SHA-256
  `626b9b422795bed4a8eef9ac272efd3e4c4eab06c7691a5e18d7b403b0edcba0`; prompt
  `.review/workflow-final-review.md` SHA-256
  `5c23b7f56a156782d9134f580792ac556ffec7e28ea9feca830c8f2bc85f751b`.
  Vendors owed: Codex (`gpt-5.6-sol/max`), Claude (`opus/max`), Grok
  (`grok-4.5/max`). Expected result and receipt directory: `/tmp/codex-runs/`.
  Historical per-vendor state at preparation: Codex=PENDING, Claude=PENDING, Grok=PENDING.
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

1. Resume Plan 6 with its Design Ready pack and one bundled Red packet, using Sol/high through
   design, implementation, and final settlement.

## Verification

- Round-1 pre-dispatch: `bash -n`, `git diff --check`, watchpoints, and Grok low smoke passed.
- Final: wrapper negative probes, tool-free Fable smoke, isolated Grok smoke, mode-aware
  manifest, bound receipts, `bash -n`, `git diff --check`, watchpoints, and Final4
  three-vendor zero-C/I confirmation passed.
