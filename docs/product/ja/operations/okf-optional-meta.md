---
type: Runbook
title: "OKF v0.2 任意メタ（sources / generated / verified）"
description: "product docs に sources・generated・verified をいつ・どう書くか。必須にはしない。"
language: ja
translation_key: operations.okf-optional-meta
status: stable
revision: 1
---

# OKF v0.2 任意メタ（sources / generated / verified）

状態: 著者向けガイド。これらのフィールドは **任意** です。未記載の既存概念も
IoTKit product ゲートでは有効です。

プロセス上の鮮度（同一 PR での product docs 更新、日英 `revision`）は引き続き必須です。
ここに書く frontmatter は、エージェントや reviewer 向けの追加の機械可読シグナルです。

## いつ書くか

**残る製品事実**が変わるときに、任意の OKF 家族を追加または更新します。

- 公開 wire / custody の **契約**本文
- 現場手順が変わる **runbook**
- コンポーネント所有を定義し直す architecture の主張

次の場合は書かなくてよいです。

- 誤字・リンク・体裁のみ
- 製品から見えない内部リファクタ
- 意味が変わらない翻訳揃え（ただし `revision` は上げる）

## 最小例

IoTKit 必須スカラーは残します。OKF 家族はフル YAML のネストで書けます（チェッカーが受理します）。

```yaml
---
type: Contract
title: "…"
description: "…"
language: ja
translation_key: contracts.example
status: stable
revision: 4
generated: { by: human:your-handle, at: 2026-08-01T12:00:00Z }
verified: { by: human:your-handle, at: 2026-08-01T12:30:00Z }
sources:
  - id: schema
    resource: https://example.invalid/path-to-schema-or-fixture
    title: 共権威の schema や設計メモ
---
```

actor 規約（OKF）:

- 人: `human:<id>`
- 自動化: `process:<id>`
- エージェントやツール: `<producer>/<version>`

`verified` は単一 mapping でもリストでも構いません。定義をコードや共権威と照合したなら
`human:` を優先します。
各 family を載せる場合、各 source には `resource`、`generated` には `by`、各 verification
event には `by` と `at` を書きます。`at` は ISO 8601 datetime にします。

## IoTKit 必須キーとの共存

| 常に必須（IoTKit ゲート） | 任意（OKF v0.2） |
|---|---|
| `type`, `title`, `description`, `language`, `translation_key`, `status`, `revision` | `sources`, `generated`, `verified`, `stale_after`, … |

「素の OKF に寄せる」ために必須キーを外さないでください。producer profile は意図的です
（[bundle root](../../index.md)）。

## パイロット方針

全概念への一括記入は **対象外** です。価値の高い契約・runbook を次に実質更新するときに
載せるのが基本です。広いパイロットは別 issue でよいです。

## 関連

- 層と OKF との意図的差分: [product index](../../index.md)
- 公式 OKF v0.2: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md
- プロセス上の鮮度: リポジトリ `AGENTS.md`（**Keep product docs current**）
