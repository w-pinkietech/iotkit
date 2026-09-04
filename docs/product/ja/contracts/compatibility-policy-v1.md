---
type: Contract
title: "IoTKit v1 互換性方針"
description: "製品1.xにおける互換性約束、独立したversion domain、mixed-version運用、migration境界を定義します。"
language: ja
translation_key: contracts.compatibility-policy-v1
status: stable
revision: 5
---

# IoTKit v1 互換性方針

> **移行中の注記（#232 子Issue 5）。** 本方針は製品1.0.0まで効力を持たない。中央の`iotkit-edge`、custody契約、旧Output Adapter契約、Console JSONは#251 で削除した。それらに触れる行は削除済みの構成を指しており、Edge Nodeだけで完結する構成への書き直しは#250 の最終PRで行う。

状態: **製品が1.0.0に到達した時点で規範**。製品major version 1の互換性約束を定義し、
0.x releaseを遡って互換seriesにするものではありません。

## 1. Release artifactとversion domain

Cargo workspace全体を一つの製品versionで表します。Release tagとGitHubが生成する
source archiveがrelease artifactです。Tagの`workspace.package.version`が、
`testdata/compatibility/v1/release-manifest.json`に記録した独立contract/storage
versionをartifactへ結びます。このmanifestは製品versionを重複して持ちません。
Checkerはunknown key、必須domain欠落、duplicate ID、空の必須evidence category、unsafeまたは
symlink経由でsource tree外へ出るpath、source treeにないpath、記録値とlisted migration
directoryの最大numeric migration versionが異なるstorage schema versionをfail closedします。
移動する`master` linkはrelease evidenceではありません。

次のversionは独立です。製品minor releaseがすべてを変える必要はなく、一つの変更が
他の変更を黙って意味しません。

| Domain | Version unit | Public authorityとevidence |
| --- | --- | --- |
| Device ingest | `/api/v1`とJSON `Envelope` / `EnvelopeAck`契約 | `ingest-v1.md`、`iotkit-ingest-contract`、fixture、test |
| Input Adapter | `adapter_api_major`と`config_schema_version` | `input-adapter-v1.md`とcompile-time host API |
| MQTT Output Adapter | MQTT topic major `iotkit/v1`とpayloadのfield集合 | `mqtt-output-adapter-v1.md`、`testdata/observation/v1`のschemaとfixture、producer / consumerのconformance test |
| Persistent storage | Storeごとのmigration/schema number | Release manifest内のNode SQLite、IoTKit Edge SQLite/PostgreSQL evidence |

Manifestはindexであり、第二のcontract authorityではありません。対になった製品文書、
code/schema、fixture、conformance testが不一致ならcontract defectです。

## 2. Public surfaceとsame-release surface

製品1.xでは次がpublic compatibility surfaceです。

- `/api/v1`の認証付きHTTP ingest
- Edge Node MQTT custody v1 topicと、独立versionのdescriptor body schemaを含む
  exactな対応payload schema
- Input Adapter API major 1と記載されたconfiguration schema
- Version付きOutput Adapter ID、route configuration、payload

OpenAPIにないConsole JSON route、server-rendered HTML、DOM、CSS、form action、
Edge Node private control API、人間向けCLI表示、private Rust typeは、独立client contract
ではなく**same-releaseのみ**です。対応する製品releaseとともに変更できます。新しい
public contractは、version付きauthorityとevidenceを持つまで互換性約束を得ません。

## 3. 製品1.xのsupport window

1.0.0以降は次のとおりです。

- 製品**minor** releaseはbackward-compatibleなpublic behaviorを追加でき、compatible
  fixも含められます。
- 製品**patch** releaseはcompatible fixを含み、意図してfeatureを追加しません。
- 両方ともsupport中のpublic contract majorを維持し、下記evolution ruleを満たす追加だけが
  compatibleです。
- Public contract major v1は製品1.x全体でsupportし、削除・非互換置換は次の製品major
  より前には行いません。
- Fixは最新製品releaseへ提供します。古い1.x binaryへのcalendar support window、
  backport、security-fix SLAは約束しません。
- Deprecated v1 behaviorは告知したnext-major removalまで文書化・testします。

これは現行pre-1.0 ruleと異なります。`0.MINOR.0`は意図的に互換性を変えられ、
`0.MINOR.PATCH`はcompatible fix用です。両期間のruleは`RELEASING.md`を参照します。

## 4. Evolutionとunknown input

Version 1は、すべてのdecoderが同じ拡張方法を持つという約束ではありません。

| Boundary | v1内でcompatibleなevolution | Breaking evolution |
| --- | --- | --- |
| Tolerant HTTP ingest object | Release済みv1 readerがignoreし、sender/receiverの意味が一意と示せる場合だけoptional fieldを追加可能。 | Required data、enum/tag value、意味、URL majorの変更はnew contract version。 |
| Strict MQTT custody payload / Adapter configuration・payload | 暗黙field追加はなし。 | Field、record family、enum、schemaの変更は明示的なnew payload/configuration/Adapter version。 |
| Console OpenAPI schema | OpenAPI schemaとsupport中readerが許すoptional documented fieldだけ追加可能。 | Closed schema、required field、enum、operation削除はnew public version。 |

現行ingest v1 Rust objectはotherwise-validなunknown object memberを意図的にignore
しますが、保存・再出力はしません。Unknown enumまたはtagged variant valueはdecodeに
失敗します。`/api/v1`にはrequest-body schema-version negotiationがなく、未対応
API-version pathは別のingest contractではありません。このtoleranceはingest固有で、
他surfaceの包括的ruleではありません。

Unknownな明示contract versionはfail closedします。Receiverはfuture versionのfield、enum、
topic、payload、configuration、database解釈を推測してはいけません。

## 5. Mixed-version運用

| 製品1.xでの組合せ | 約束 / operator action |
| --- | --- |
| 既存ingest v1 client → 新しい1.x Edge Node | Supported。 |
| Supported custody v1 payloadを送る既存Edge Node → 新しい1.x IoTKit Edge | Supported。 |
| 新しいEdge Node → 古いIoTKit Edge | Guaranteeしない。先にIoTKit Edge、次にEdge Nodeを更新する。 |
| 既存v1 Output Adapter consumer → 新しい1.x IoTKit Edge | 同じversion付きAdapter IDとexact payload contractならsupported。 |
| 古い`nodectl`またはdirect database access → 新しいNode schema | Unsupported。対応するreleaseのtoolを使い、direct database mutationを互換pathにしない。 |
| API major 1向けにcompileしたInput Adapter → 新しいEdge Node | Source/configuration contractがv1である間はsupportするが、AdapterはEdge Node releaseとともにrebuildする。 |

MQTT PUBACKはBroker receiptだけです。Durable IoTKit acceptanceを表さず、単独では
mixed-version pathを安全にしません。

## 6. Storage migrationとrollback

各release manifestは、そのsource archiveに対応するNode SQLite、IoTKit Edge SQLite、
IoTKit Edge PostgreSQLのexact schema evidenceを記録します。Release済み1.x database
schemaは、後続1.x releaseへのtested forward migrationを持たなければなりません。
Down migrationとimage-only rollbackは約束しません。

Update失敗時はnew binaryを停止し、暗号化した更新前backupを保持し、そのbackupを
new candidateへrestoreして、matching old binaryへswitchします。Migration済みdatabaseを
old binaryで開いてはいけません。SQLite-to-PostgreSQL migrationはcurrent release schema
だけを受理します。古いSQLite sourceは先にcurrent IoTKit Edgeでmigrateし、停止してから
empty PostgreSQL targetへcopyします。

Pre-1.0 databaseとbackup artifactは、現在test済みのrunbookだけが対象です。製品1.xの
preservation promiseではありません。

## 7. Breaking changeと緊急時のprocess

計画された非互換変更の前に、maintainerは次を実施します。

1. 理由、対象version domain、supportするold/new range、upgrade order、removal targetを
   issueとcurrent authorityへ記録する。
2. Old inputをreinterpretせず、new version、path、topic、Adapter ID、schemaを明示導入する。
3. 英日文書、type/schema、fixture、conformance test、必要なmigration/dual-read evidence、
   更新済みrelease manifestを同梱する。
4. Old supported versionを削除する前に、migration/deprecation noticeをrelease noteへ出す。

Securityまたはdata-loss防止の緊急変更は、この通常期間より先にfail closedできます。
対象version、safeなdata保持、recovery/migration actionを示し、release noteとproduct
authorityへ記録します。これはavailabilityの例外であり、customer dataを黙って
reinterpretまたはdiscardする許可ではありません。

## 8. 明示的なnon-goal

この方針は未公開pre-v1 behaviorをすべて永続保存しません。これだけでConsole routeを
すべてOpenAPI APIにしたり、独立したdescriptor validatorを統合したり、external consumer
gateやhistorical golden databaseを追加したりもしません。これらはこの約束を広げる前に
別issueとcompatibility evidenceを必要とします。
