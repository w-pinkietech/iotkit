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

Contract and runbook documents in this bundle are orientation guides. Their links
identify the detailed contract artifact or operator runbook in the source
repository; the shortened OKF text does not replace those authorities. Historical
plans, review transcripts, local machine details, and customer configuration do not
belong in this bundle. `scripts/check-okf-docs.mjs` enforces this repository profile.
