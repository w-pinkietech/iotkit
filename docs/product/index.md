---
okf_version: "0.2"
---

# IoTKit product documentation

* [日本語](ja/index.md) - IoTKitの製品モデル、構成、公開契約、導入・復旧の正本への入口。
* [English](en/index.md) - Entry point for the IoTKit product model, architecture, public contracts, and operations.

# Authority and format

This tree is the **current human-readable product corpus** (product documentation).
It is packaged as an [Open Knowledge Format (OKF) v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundle so agents and tools can consume a portable markdown-plus-frontmatter layout.

IoTKit uses a small **producer profile** on top of OKF v0.2:

- Every product document, including Concepts, Architectures, Contracts, and Runbooks, has Japanese and English counterparts at the same relative path.
- Both share `translation_key`, `type`, `status`, and a positive integer `revision`.
- A content change must update both translations and increment their shared revision.
- `language` records the file locale and must match the path (`ja/` or `en/`).

Those bilingual and revision rules are repository extensions (OKF allows producer-defined keys).
OKF v0.2 itself only requires `type` on each concept; optional provenance/trust/lifecycle
fields (`sources`, `generated`, `verified`, `stale_after`, …) may be added later without
changing the authority path.

Versioned machine schemas, exported wire types, shared fixtures, and conformance tests
remain co-authorities for their contracts; a disagreement is a contract defect rather
than permission to follow one artifact silently. Historical plans, review transcripts,
local machine details, and customer configuration do not belong in this tree.
`scripts/check-product-docs.mjs` (and the compatibility entry `scripts/check-okf-docs.mjs`)
enforces this repository profile.
