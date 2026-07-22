---
okf_version: "0.1"
---

# IoTKit knowledge bundle

* [日本語](ja/index.md) - IoTKitの製品モデル、構成、公開契約、導入・復旧の正本への入口。
* [English](en/index.md) - Entry point for the IoTKit product model, architecture, public contracts, and operations.

# IoTKit OKF profile

This bundle uses a deliberately small OKF v0.1 producer profile. Every concept has
Japanese and English documents at the same relative path, with the same
`translation_key`, `type`, `status`, and positive `revision`. A content change must
update both translations and increment their shared revision.

The documents in this bundle are the complete human-readable current product corpus.
Versioned machine schemas, exported wire types, shared fixtures, and conformance tests
remain co-authorities for their contracts; a disagreement is a contract defect rather
than permission to follow one artifact silently. Historical plans, review transcripts,
local machine details, and customer configuration do not belong in this bundle.
`scripts/check-okf-docs.mjs` enforces this repository profile.
