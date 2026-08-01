---
okf_version: "0.2"
---

# IoTKit product documentation

* [日本語](ja/index.md) - IoTKitの製品モデル、構成、公開契約、導入・復旧の正本への入口。
* [English](en/index.md) - Entry point for the IoTKit product model, architecture, public contracts, and operations.

# Authority, format, and gate

Three layers—do not mix them when diagnosing failures:

| Layer | Name | Role |
|---|---|---|
| **Authority** | Product documentation | This tree (`docs/product/`) is the human-readable current product corpus |
| **Format** | OKF v0.2 packaging | Portable markdown + YAML frontmatter ([SPEC](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)) |
| **Gate** | IoTKit product producer profile | Repository CI rules on top of OKF (bilingual pairs, revisions, closed links, …) |

`docs/okf/` is only a compatibility stub that points here. OKF is the *format*, not a
second corpus and not the name of the CI gate.

## IoTKit product producer profile

Required on every product document—Concept, Architecture, Contract, and Runbook
(repository extensions; OKF allows producer-defined keys):

- Japanese and English files at the **same relative path**
- Shared `translation_key`, `type`, `status`, and positive integer `revision`
- Content changes update **both** translations and increment the shared `revision`
- `language` matches the path locale (`ja/` or `en/`)
- `title` and `description` present

OKF v0.2 itself only requires `type` on each concept. Optional OKF families
(`sources`, `generated`, `verified`, `stale_after`, …) may appear as nested YAML
without changing the authority path; the checker parses full YAML and accepts
unknown keys.

## Intentional differences from plain OKF consumers

The official OKF consumer rules are deliberately tolerant (missing optional fields,
unknown types, broken links must not reject a bundle). **This repository’s product
gate is stricter on purpose** so the corpus stays a closed, bilingual product authority:

| Topic | OKF (typical consumer) | IoTKit product gate |
|---|---|---|
| Broken **in-bundle** links | Must not reject the bundle | **Fail** (product docs should form a closed graph) |
| `log.md` | Allowed reserved file | **Not used** (history lives in git; checker forbids it) |
| `type` values | Free strings; unknown types tolerated | **Allow-list:** Concept, Architecture, Contract, Runbook |
| Extra frontmatter keys | Allowed extensions | Allowed, including nested/list values; required extensions are listed above |
| Root `okf_version` | MAY declare | **Must** be `"0.2"` on this bundle root |
| Path layout | Free | Concepts under `ja\|en` × `concepts\|architecture\|contracts\|operations` |

Co-authorities for versioned contracts (schemas, fixtures, tests) sit outside this
bundle; disagreement is a contract defect, not permission to follow one artifact
silently. Historical trees (`docs/redesign/`, `docs/superpowers/`) are not authority.

Install the pinned checker dependency after a fresh checkout or package-lock change:
`npm ci --prefix scripts/docs`.

**Checker:** `node scripts/check-product-docs.mjs` (compatibility entry:
`scripts/check-okf-docs.mjs`).

| Mode | Command | Checks |
|---|---|---|
| `all` (default, CI) | `node scripts/check-product-docs.mjs` | OKF min + IoTKit product profile |
| `okf-min` | `--mode=okf-min` | frontmatter + non-empty `type` + root `okf_version: "0.2"` |
| `iotkit-product` | `--mode=iotkit-product` | bilingual pairs, revisions, type allow-list, closed in-bundle links, … |

Failures print a layer tag (`[okf-min]` or `[iotkit-product]`).