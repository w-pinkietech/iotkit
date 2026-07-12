# Active Development Ledger

This is persistent workflow state, not design authority. Verify every factual claim against
git/disk/test. Never store secrets here.

## Reality

- Repository: `iotkit-next`
- Branch: `master`
- Artifact/base HEAD: `f10e95d`
- Working tree at workflow-design start: user-owned untracked
  `docs/eval/autonomous-development-policy-discussion-2026-07-11.md`
- Active phase: hybrid local-Main/Codex-Cloud candidate-lane harness is under Large/Red workflow
  review before Plan 6 Task 5; product code remains worker-only

## Mission

- Adopt risk-adaptive autonomous delivery with Design Ready, Green/Yellow/Red, independent
  review, and persistent restart state.
- Preserve independent review, TDD, verification, design canon, and product-code worker
  separation.
- Deliver Plan 6 as Large / Red under Sol/high through final settlement.
- External push/PR/release is not authorized by this mission.
- For the 2026-07-12 context-migration mission only, the user explicitly authorized the proposed
  sequence including final pushes and freezing the former design repository. This does not grant
  standing push/PR/release authority for Plan 6 product tasks.

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
- 2026-07-12: Approved the bundled Plan 6 Red recommendations: preserve logical gateway
  identity/TLS/device-token continuity on restore and accept loudly reported post-backup
  device-token revocation rollback; invalidate admin/operator/session authority; allow factory
  reset only from SSH or physical/local root; keep initial ownership local-CLI-only and defer
  batch provisioning to a separate pre-distribution deliverable.
- 2026-07-12: Record declarative provisioning for multiple sensor devices on one gateway as an
  **unsettled future candidate**, separate from both Plan 6 and the gateway-box/card distribution
  deliverable. Candidate note:
  `docs/superpowers/future/device-fleet-declarative-provisioning.md`. It may inventory non-secret
  network intent and generate device-specific work, but preserves credentials per independent
  authenticated network sender, forbids sharing across independent senders, requires explicit R14
  apply, and keeps listener enablement separate, DHCP/router authority external, and retire
  non-automatic. QR codes are not a requirement; they were only a discussion example.
- 2026-07-12: Starting with the next task, measure and report wall time for planning/research,
  implementation, verification, independent review, finding fixes/reverification, documentation/
  commit, and total elapsed time. Distinguish tool/model wait from direct Main work where the
  evidence permits; record model/effort and retry/confirmation counts.
- 2026-07-12: The timing-rule change modifies workflow authority and is therefore classified
  **Large / Red** by the workflow promotion rule. User authority is the explicit request to adopt
  per-step timing; scope is measurement/reporting only, with no product or review-strength change.
  It requires Sol/high independent review before its separate documentation commit.
- 2026-07-12: Before Task 5, consolidate design authority and implementation context into
  `iotkit-next` so one local or Codex Cloud clone can resume work. Import `docs/redesign/` with
  history, move active references and restart guidance in-repository, avoid submodules/two-way
  sync, publish the consolidated repository, and freeze `iotkit-redesign` with a destination
  pointer. This authority-location change is **Large / Red**; the user's explicit approval settles
  the value decision, while implementation still requires Sol/high independent review.
- 2026-07-12: Adjust the repository so local Main can dispatch/monitor/collect Codex Cloud candidate
  work and Cloud can temporarily act as Main while the computer is unavailable. This workflow and
  external-dispatch change is **Large / Red**. The approved envelope keeps Cloud work on candidate
  branches, preserves separate authority for each future Cloud submission/push/PR/merge/release,
  and forbids task URL/status/diff from satisfying hash-bound review settlement. Design record:
  `docs/superpowers/migrations/2026-07-12-hybrid-cloud-execution.md`.
  Initial Sol/high review found that the installed CLI cannot prove the commit actually checked
  out by Cloud. The implementation was narrowed to disable automatic apply, label requested base
  as unverified, snapshot the exact prompt/query, preserve pre-dispatch pending state, and recover
  interrupted locks before confirmation review.

## Review state

- Hybrid local/Cloud execution harness: **NOT SETTLED**. Design record, wrapper, environment setup,
  negative tests, workflow authority, Cloud guide, and active ledger require a confirmation
  Sol/high review on one final manifest before commit. Initial review returned C0/I7/M1; all seven
  Important areas were reconciled in the narrowed design. Initial result SHA-256
  `7e61273cb559f9802e1759d93b5b2b8321c52b50b4116c8a7d1ce8ceca6a2b0e`; receipt SHA-256
  `fcc93e7ed64dd772d671285b592b3897aae59a292794ebda27ba9234dcb28dd4`; Sol/high review ran
  2026-07-12T04:43:50Z–04:48:18Z. First confirmation returned C0/I5/M0 and drove exact-query
  binding, control-character rejection, single-attempt confinement, HMAC/schema validation, and
  pre-commit review-order fixes. Confirmation result SHA-256
  `ab3a4920488e3260865928541bc00be99021e0c148b6431dba904fa9e01f2936`; receipt SHA-256
  `466573849dd237a75bbd5bee4325589a80dbd618e4a7b400433870557fc9159d`; Sol/high ran
  2026-07-12T05:04:26Z–05:11:26Z. The next final round returned C0/I1/M0: all prior findings were
  closed, but PID-based stale-lock recovery retained a concurrent TOCTOU. Result SHA-256
  `ea65ef2a47b80a5889bc4c8d52255c8e364dde41cc250c38e7c7f1a2578d8c98`; receipt SHA-256
  `9701d5c2c83e55b53f4e3d1d9ddfec37366483978bafa4e70ffc12ac8ee2184b`; Sol/high ran
  2026-07-12T05:16:36Z–05:22:08Z. The fix replaces PID recovery with kernel-held `flock` and a
  non-authoritative interruption marker. Its confirmation returned C0/I1/M1 because the wrapper
  alone held the lock, allowing a surviving orphan CLI after wrapper death; it also requested a
  distinguishing live-remote-movement test. Result SHA-256
  `3d55a23a09e199fe568ed505d4c334b423cb310c467a4a53cc36eab7bb07225d`; receipt SHA-256
  `04b9af031d9f3142bb0360ab27110acafb7dc110e1c1c8c3d28a1f2c231de1f9`; Sol/high ran
  2026-07-12T05:25:12Z–05:30:55Z. A guardian now retains `flock` until an orphan CLI exits and the
  remote-movement regression is included; one final exact-hash confirmation remains owed. No
  Cloud task was submitted by this change.

- Single-repository context migration: **SETTLED and published as two-parent merge `f10e95d`**.
  Mission/design record:
  `docs/superpowers/migrations/2026-07-12-single-repository-context.md`. Artifact/base
  `f8186ea`; source design commit `f10c2a5`; history-preserving split commit `28fe683`.
  Fresh Codex Sol/high review manifest `.review/context-consolidation.manifest` SHA-256
  `e408b0171a603396faf9c51b25b25b90e44d189db0f2f2ab329220f4d2fab97e`; prompt
  `.review/context-consolidation-review.md` SHA-256
  `194c3607520ffa1d53770d814c55449903e255667152561d6b11eb64c7e27e6f`.
  Initial result `/tmp/codex-runs/codex-context-consolidation-review-20260712-125311-387886.txt`
  returned `Critical 0 / Important 3 / Minor 0`; all three findings were resolved by the final
  confirmation: include this ledger in the final bound artifact, stage every migration
  file before confirmation/commit, and make pre-publication rollback preserve unrelated files.
  Result SHA-256 `af64e233580149e4e9fb9f17b42460d39043dde14cc4fd2289530ba1b2efd47f`;
  receipt SHA-256 `2aeb6c370f3cba632604def948d15504d6421da27e54d97df3ba8a110b0264fb`.
  Final confirmation manifest `.review/context-consolidation-confirm.manifest` SHA-256
  `d5bd1b37d74e8bdba3c5b150cb622fe2de59c3612bee2a16ea117d4536acf373`; prompt
  `.review/context-consolidation-confirm-review.md` SHA-256
  `bb0fda4b7881391135c4bfae0a978e11918c1ae84e45e9d7529e6e6c3f84f191`.
  Fresh Codex Sol/high returned `Critical 0 / Important 0 / Minor 0`; result SHA-256
  `ae215588c33ae613b44716d2d745c870a4ebe366a3f19054b050f47525bcf29c`; receipt SHA-256
  `4d265466ddb1e0c0a89d3f842157cbb0dcb6a3451f43bd9907d63eb8663d06a3`.
  GitHub `iotkit-next/master` was verified at `f10e95d`. The former repository pointer was
  independently confirmed at zero C/I, committed as `iotkit-redesign` `538f9c3`, pushed, and
  GitHub reported `isArchived=true`. Its final confirmation manifest SHA-256 is
  `c5b03ada2706f95fe190218320219ad956ecab8f3de6335ce9410dbfc05988a3`; prompt SHA-256
  `7915caa6e325ee3e517246b46451276efe6b95fade87d6b8c360c58759e21888`; result SHA-256
  `66738f225413654e57cd4c2d00bba2f707e5abb065b0fe8e1cb49ed7de6d7472`; receipt SHA-256
  `facee5043fe8a93b7ccfc2909eecebb683b034b3cf7a172e03b50ea02088960f`.

### Single-repository migration timing

- Boundary: `2026-07-12T12:45:06+09:00` to `2026-07-12T13:09:51+09:00`; total **24m45s**.
- Planning/research: **2m00s**; documentation implementation: **4m30s**; verification:
  **1m35s**; independent review: **12m47s**; finding fixes/reverification: **2m22s**;
  documentation/commit/push/archive: **1m31s**. Categories are non-overlapping and sum to the
  total. The first three internal boundaries are reconstructed estimates; the other non-review
  categories allocate exact inter-review intervals from the recorded actions rather than wrapper
  measurements. Wrapper review times, Git commit times, and the total boundary are exact.
- Model/effort: four fresh Codex reviews, all `gpt-5.6-sol/high`; two were confirmation rounds.
  Initial migration review found 3 Important; its confirmation found zero. Initial freeze-pointer
  review found 1 Important release-order issue; its confirmation found zero. No worker retries or
  product implementation runs occurred.

- Plan-6 Task 4 default-off site-LAN HTTP listener foundation: **SETTLED and committed as
  `d11bc8d`**. Final manifest `.review/plan6-task4-confirm3.manifest` SHA-256
  `1f7a4f07edb24233f43882dc5b55482b18f783595ff8853f264980bafab2cf52`; prompt
  `.review/plan6-task4-confirm3-review.md` SHA-256
  `1b7d333aabf182445c2bc6e367209f93191ce31881514e1bc49169c9d6ea9c29`.
  Fresh Codex `gpt-5.6-sol/high` returned `Critical 0 / Important 0 / Minor 0` and declared the
  task commit-ready; result SHA-256
  `d722a25df973aec1fe6caf5e1b00be5defa757194353a4dd10ecd9cd3e1e17ac`; receipt SHA-256
  `44defbc5d59c7d69556fa6c9e81ef7cb418760666d676506007b892a39c4c624`.
  Earlier rounds found and closed 14 Important and 1 Minor issues across restore/control
  availability, TLS custody/handshake/fingerprint/atomic settlement, OS interface and route
  confinement, bind/switchover TOCTOU, continuous authority, truthful desired/applied health,
  R14 secret redaction/schema strictness, and executable layer/transport tests. Main reran
  `scripts/verify.sh` after the final fixes; workspace tests, real socket/OS inventory probes,
  Clippy `-D warnings`, layer checks, formatting, and diff checks passed. Claude/Grok unavailable
  and not required.
  Retrospective Task wall time was 1h34m01s (10:55:25–12:29:26 JST, dispatch start through
  commit completion): initial implementation 25m08s; four independent review rounds 18m49s;
  three principal fix rounds plus one environmental-test correction 41m58s; remaining Main
  verification/manifest/commit critical-path time 8m06s. This retrospective predates the formal
  timing rule, so its phase split is receipt-derived rather than a live phase log.

- Plan-6 Task 3 device credential lifecycle/capacity/recovery foundation: **SETTLED and committed
  as `166d50c`**. Final manifest `.review/plan6-task3-confirm3.manifest` SHA-256
  `3f404fe1367278a5c143b9da3fa2443b48a8cd6ef938e219e5d20637bf40ac3c`; prompt
  `.review/plan6-task3-confirm3-review.md` SHA-256
  `c67ed2df290b231d54d5fbbd63676c9e86dcb8448571cc52db0a04a94a32b7b6`.
  Fresh Codex `gpt-5.6-sol/high` verified every manifest blob, confirmed all prior findings closed,
  and returned `Critical 0 / Important 0 / Minor 0`; result SHA-256
  `d1f4d7257138cac50f3cfb0b2bcebe7291fec2822fb2c8f3e1b5aed42c3c11f0`; receipt SHA-256
  `f13f3dc237dafb92634719fd63c44e01c4d2b8f069915bf9d5af547d0bdfcdd5`.
  Earlier fresh rounds found and closed 26 Important and 4 Minor issues across restore/token
  continuity, typed secret ownership, R14 mutation boundaries, bounded authentication/scopes,
  generation fencing, lifecycle clocks, live-authority capacity/debt math, overflow handling,
  atomic consent previews, snapshot no-clobber publication, health false-safe states, and CLI
  recovery UX. Main reran `scripts/verify.sh` after the final fixes; workspace tests (including
  real loopback API tests), Clippy `-D warnings`, layer checks, formatting, and diff checks passed.
  Claude/Grok unavailable and not required.

- Plan-6 Task 2 wire/principal collector seam: **SETTLED and committed as `82a71f6`**. Confirmation
  manifest `.review/plan6-task2-confirm.manifest` SHA-256
  `046f2679118b489374274c718977c8ff5f13b19e4f26a442731472fc804a588a`; prompt
  `.review/plan6-task2-confirm-review.md` SHA-256
  `5909b76e57ca852c8053025dba055ee041ae65aae65cc5315ff6cd677305e7b2`.
  Fresh Codex `gpt-5.6-sol/high` confirmed the four discovery findings were closed and returned
  zero Critical/Important; its sole Minor was this ledger refresh. Result SHA-256
  `b68c76badf1f5c745e2a246838ac6a79a84fd71e907cf6929ea37284a2fe6368`; receipt SHA-256
  `010a2c233de82a7c700924922e8ef8ec3a9d58533e1299b8f0764c062f645038`.
  Discovery manifest `.review/plan6-task2.manifest` SHA-256
  `913142a9bedb28539ea02c21e021bcaa750a2c1ea8aa453a606afe5600ac7788` found four Important
  gaps: sender-created trusted principals, suppressible scope signals, unchecked freshness
  limits, and stale `age_ms` documentation. All were fixed through a Sol/high product-code
  worker. Main reran `scripts/verify.sh`, including real loopback gateway tests; all passed.
  Claude/Grok unavailable and not required.

- Plan-6 Task 1 ownership/recovery/clock/restore closure: **SETTLED**. Final manifest
  `.review/plan6-task1-final.manifest` SHA-256
  `c348b62d152c7e75e390ca55a16cb33a414abd137e50da9b4161c5e6d989e66d`; prompt
  `.review/plan6-task1-final-review.md` SHA-256
  `eee401d1c6570608f830b5067fce0ba9e0df52391e3112ba57e98489c967d9f5`.
  Fresh Codex `gpt-5.6-sol/high` confirmed the last restore-marker/time-confirmation fixes and
  returned zero unresolved Critical/Important after the full residual scan. Result SHA-256
  `4432cb82fc3d47d3f31dce2464beca8d82d88526abc42bf5494f307c5e940685`; receipt SHA-256
  `ad2a1b4240a8d60098e503f54e32afb4e78ae650bb5abd8bddc0111d35bce07e`.
  Discovery found one Critical old-passphrase/reset race and seven Important TLS/schema/clock/
  TTY/restore/provenance/docs gaps; confirmation found two further Important marker/time TOCTOU
  gaps. All were fixed through Sol/high product-code workers. Main reran real TCP API integration,
  gatewayctl PTY/restore tests, and `scripts/verify.sh`; all passed. Claude/Grok unavailable and
  not required.

- Plan-6 formal specification: **SETTLED**. Final staging/canon-aligned manifest
  `.review/plan6-spec-staging-final.manifest` SHA-256
  `678f8e505fafb0bc12c9de60d631a9a91f1d0def7f58f69a321ac883a83e8dd5`; prompt
  `.review/plan6-spec-staging-final-review.md` SHA-256
  `e342eca69a170d77e43b2e96ee485a583d905ac5481504550ca6514742f4d546`.
  Fresh Codex `gpt-5.6-sol/high` returned zero unresolved Critical/Important. Result SHA-256
  `d3613dde722a147924e3066aabf50f3997b4aac401e766e23d8c9810728d0942`; receipt SHA-256
  `c48421ceb9a81f4bf1ed366feafbf7e08b5c7e3fc19542f39e64af5e63d873a8`.
  Earlier rounds found and closed 16 Important issues covering pre-auth DoS/accounting,
  subject/ack semantics, compatibility/validation/TLS UX, restore/dedup/clock ownership, code
  placement, and staging overflow alignment. Claude/Grok unavailable and not required.

- Plan-6 design-canon propagation: **SETTLED and committed in the parent design repository as
  `f10c2a5`**. Final manifest `/tmp/plan6-canon-confirm.manifest` SHA-256
  `76367eb58039621f133c0ffb7e1a8665cb95da38af4f19872cee8a6253864bfc`; prompt SHA-256
  `59c99ac4261060825a78f37d80ce33aee3ea43af38efc4d0d87554a83de1316e`.
  Fresh Codex Sol/high returned zero C/I after closing five Important propagation gaps. Result
  SHA-256 `d3613dde722a147924e3066aabf50f3997b4aac401e766e23d8c9810728d0942`; receipt SHA-256
  `109026a4af401f2fb465280d2cb33b175548ed80235fc200a2c163a78ec917e8`.

- Plan-6 contract-centered implementation plan: **SETTLED**. Final substantive manifest
  `.review/plan6-implementation-plan-confirm.manifest` SHA-256
  `0672bd86f9ce909632ec757f763dea986a7a27376271519df0032287834a09c6`; prompt
  `.review/plan6-implementation-plan-confirm-review.md` SHA-256
  `92089c9a1fcbf43e9af5e954e3fb9d02587e9c1d1215bd8cbc3e5f3577f7ac16`.
  Fresh Codex Sol/high confirmed all eleven initial Important sequencing/verification findings
  closed and returned zero C/I. Result SHA-256
  `d3613dde722a147924e3066aabf50f3997b4aac401e766e23d8c9810728d0942`; receipt SHA-256
  `f954fdf011fa2150ed165ec161d74b357b9ed0a5b0047443a63704794c60b425`.
  Status-label confirmation manifest `.review/plan6-plan-settled.manifest` SHA-256
  `da362dd4e75d1f4d7c953902972893b7fb542b493753c78a6a3da9927f600b8f` returned zero C/I;
  receipt SHA-256 `aaf39c554376a7d8e33cd4660638716c924e272744da8537c8157592e32057b1`.

- Plan-6 Design Ready zero-C/I confirmation: **SETTLED**. Manifest
  `.review/plan6-design-ready-final3.manifest` SHA-256
  `048012f9f6c4536be97af41d3acae54cfff771f955def27de4a150b463ed9c0a`; prompt
  `.review/plan6-design-ready-final3-review.md` SHA-256
  `33e7e02fd6bb89d8a8b78b1ea67f95b5b16b1fc45996a6bee8f29f0b0e6cd112`.
  Fresh Codex `gpt-5.6-sol/high` confirmed both final prescriptions and returned zero unresolved
  Critical/Important in the residual scan. Result SHA-256
  `6cdc9d23b7eded7972a256b553c99299bc471ffe35338d66c59d1d575d221c7c`; receipt SHA-256
  `a0696676007d852bc84560faba1f4e01107be49e21faf4b68737457806924858`.
  Claude/Grok unavailable and not required.

- Plan-6 Design Ready exact final confirmation: **COMPLETE, NOT SETTLED**. Manifest
  `.review/plan6-design-ready-final2.manifest` SHA-256
  `63ec44d674819d4e67dd56881adc34139abad325ea03c156d040b96c5d442d22`; prompt
  `.review/plan6-design-ready-final2-review.md` SHA-256
  `a32bb468b1fb68995bd2134d270c5a056f17f0789672b6fb1d17960f02f4f280`.
  Fresh Codex `gpt-5.6-sol/high` confirmed both owned findings, then found two new Important
  internal contradictions: the device-token provenance row ignored the selected restore mode,
  and the actual echoed admin input contradicted secret-safety traceability. Both exact fixes
  are applied; Plan 6 now requires non-echoed local passphrase input. Result SHA-256
  `135fd4c7dee8c4e3cee726472de17656faa50048e23b9809cff64afbbb16da6e`; receipt SHA-256
  `d3d4cc37b688108c2359993d2cfe57e66e9ef375335f8e2449d143a978bb79aa`.
  Claude/Grok unavailable and not required.

- Plan-6 Design Ready final confirmation: **COMPLETE, NOT SETTLED**. Substantive manifest
  `.review/plan6-design-ready-final.manifest` SHA-256
  `e03cb60ba0f19c648dd8098289c3c69b5ffd6b0f0bf516360c4de0e2075ce29d`; prompt
  `.review/plan6-design-ready-final-review.md` SHA-256
  `7bca7615c30142ef4ecefdf07b88ac26e15a2ba05da661086f8a1d84dc08d6e4`.
  Fresh Codex `gpt-5.6-sol/high` confirmed round-1 items 2–10 and found two remaining Important
  omissions: D2 §3.5's active operator-token restoration conflict was not named, and DB/TLS
  restore lacked a cross-filesystem crash fence. Both exact prescriptions are now applied.
  Result SHA-256 `022f4f4265b56efd4d5a3132677ea38dfbcac39d6022a724a871eed85df7205b`;
  receipt SHA-256 `3243abe92bf62f5d7265ef078896db47334254801922c60f82e81302e847b788`.
  Claude/Grok were unavailable and not required.

- Plan-6 Design Ready round 1: **COMPLETE, NOT SETTLED**. Substantive manifest
  `.review/plan6-design-ready.manifest` SHA-256
  `ebfe355aad145ab68a8836f2d4ee7300e418e7a18fddcd65f9e11810273aa01a`; prompt
  `.review/plan6-design-ready-review.md` SHA-256
  `a926e3b18d9b1197739adcd172477f3bd2725f0fefc0fc3eb5aad14888b2b40e`.
  Fresh Codex `gpt-5.6-sol/high` found zero Critical and ten Important gaps: restore material
  matrix/identity/TLS, current non-pristine restore predicate, preseed lifecycle, filesystem
  crash ordering, SSH reset authority, nonexistent provisioner UX, canon propagation, auth
  clock rollback, device-token reissue state, and partial TLS loss. All are addressed in the
  revised pack; Red consequences were expanded rather than silently decided. Result SHA-256
  `9f8eb68ecec26357db2205c06dccc78bb5e703ab0d79d8aa191234558b9c2ba9`; receipt SHA-256
  `eddbba5279dbc28d11d1d3e8507aa8f865e236467a852ae9b140021f3466f4ea`.
  Claude/Grok were unavailable and not required.

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

## Resolved Red packet

Recorded as `docs/superpowers/PLAN6-DESIGN-READY.md`; independent review and the user's bundled
decision are SETTLED. The packet bundles:

1. R22 restore versus auth revocation-generation rollback.
2. Factory reset meaning and authority.
3. Product scope/UX consequence of the local-only bootstrap producer and recovery limit.

Approved result: replacement preserves logical gateway identity/TLS and active device-token
continuity while accepting explicit post-backup revocation rollback risk;
admin/operator/session authority is invalidated with a new auth epoch;
factory reset is SSH/local-root full erase and distinct from non-destructive admin recovery;
Plan 6 uses local CLI ownership only and defers nonexistent provisioning tooling to the
distribution gate. These choices are now being folded into canon and the formal specification.

## Yellow decisions

- Canonical workflow location: `docs/development-workflow.md`; reversible by reverting the
  workflow commit. Reconsider if role-specific docs drift or agents fail to load it.
- Persistent state location: this file. It is deliberately Git-tracked and non-canonical for
  product design. Reconsider if update churn obscures product commits.
- Current cumulative roll-up: these workflow-location choices remain reversible and do not
  alter the approved product envelope. Reviewers must confirm this classification on the final
  workflow round.

## Next executable work

1. Settle and commit the hybrid Cloud execution harness; do not submit an external Cloud task as
   part of harness verification.
2. Prepare and dispatch Plan 6 Task 5 through Sol/high `scripts/codex.sh impl` or an explicitly
   authorized Cloud candidate task,
   with per-step wall time measurement enabled from dispatch.

## Verification

- Single-repository context migration: imported `docs/redesign` tree SHA `3548c3e` exactly matches
  old `f10c2a5`; the merge parents are `f8186ea` and split-history `28fe683`; 100 changed-file
  local Markdown targets resolved; Main and independent-review `scripts/verify.sh` passed; both
  remote tips and the old repository's archived state were verified after publication.
- Plan 6 Task 4: final manifest review returned zero C/I/M; Main-owned `scripts/verify.sh` passed
  after the final fix round, including workspace tests, real sockets/OS inventory, Clippy
  `-D warnings`, layer checks, formatting, and `git diff --check`.
- Plan 6 Task 3: final manifest review returned zero C/I/M; Main-owned `scripts/verify.sh` passed
  after the final fix round, including workspace tests and real loopback gateway integration;
  Clippy `-D warnings`, layer checks, formatting, and `git diff --check` are green.
- Plan 6 Task 2: focused contract/collector/client/adapter/registry/gateway tests and Main-owned
  `scripts/verify.sh` passed; workspace Clippy `-D warnings`, layer checks, formatting, and
  `git diff --check` are green. Confirmation review returned zero C/I.
- Round-1 pre-dispatch: `bash -n`, `git diff --check`, watchpoints, and Grok low smoke passed.
- Final: wrapper negative probes, tool-free Fable smoke, isolated Grok smoke, mode-aware
  manifest, bound receipts, `bash -n`, `git diff --check`, watchpoints, and Final4
  three-vendor zero-C/I confirmation passed.
