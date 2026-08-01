---
type: Contract
title: "Edge Node保管責任契約 v1"
description: "MQTTによるcustody移転、activation、record family、ack、retry、認証を定義します。"
language: ja
translation_key: contracts.edge-node-custody-v1
status: stable
revision: 4
---

# Edge Node保管責任契約 v1

状態: 承認済みMQTT v1 target contract。Record、descriptor、`accepted-through`、activation、publication admissionを実装済みです。旧HTTPS publisherは移行用codeとして残りますが、composition rootは起動しません。

この契約はcanonical recordが一つのEdge Nodeを出る方法と、Edge Nodeがcustodyを移転できる時点を定義します。Production、OEE、工程、alarm文、Pinikiet stateなどapplicationの意味は含みません。

## Role

- **Edge Node publisher:** durable outboxを読み、bounded batchをpublish・retryし、local delivery cursorを所有する。
- **MQTT Broker:** QoS 1 messageを運ぶ。PUBACKはBroker receiptだけ。
- **IoTKit Edge:** canonical recordと連続cursorを耐久commitしてからapplication custody ackをpublishする。Raw query、Edge scopeのsemantic/output境界も提供するが、その失敗はraw custodyを弱めない。
- **Application consumer:** Pinikiet等は別のOutput Adapter契約を受け取る。Raw custody streamを消費せず、業務成功はEdge Node purgeを許可しない。

## Topic

```text
iotkit/v1/edge-nodes/{edge_node_id}/records
iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
iotkit/v1/edge-nodes/{edge_node_id}/descriptors
iotkit/v1/edge-nodes/{edge_node_id}/activation/request
iotkit/v1/edge-nodes/{edge_node_id}/activation/result
```

`records`と`accepted-through`はQoS 1、retain禁止です。`descriptors`はQoS 1 retainedのcomplete current-state replicaで、custody streamではありません。Activation request/resultはQoS 1、retain禁止です。IoTKit Edgeはmatching resultをcommitするまでrequestを耐久retryし、MQTT PUBACKをactivation完了としません。ACLは各Edge Nodeを自身のtopicへ限定します。

## Activationとpublication admission

Broker enrollmentとEdge Node activationは別です。Enrollmentはconnection profile、static credential、exact topic ACLを与えるinstall operationです。Activationは認証付きConsole operationで、exact `(edge_node_id, ledger_epoch)` incarnationに新しいcustody streamの開始を許可します。

Enrollment済みでもinactiveなEdge Nodeはdescriptorだけをpublishし、activation requestを受けます。Recordをpublishせず、pre-activation readingへ`pub_seq`を与えたりpublication outboxへ入れたりしません。Local commissioning preview用に耐久保持できますが、後からIoTKit Edgeへreplayしません。

Admin activation transactionは、`edge_id`、unique `activation_id`、exact Edge Node/epoch、actor audit、retryable command outboxを耐久保存してから次をpublishします。

```json
{
  "schema_version": 1,
  "activation_id": "act-0123456789abcdef0123456789abcdef",
  "edge_id": "edge-0123456789abcdef0123456789abcdef",
  "edge_node_id": "edge-node-01",
  "expected_ledger_epoch": "01J...",
  "grant_revision": 1,
  "issued_at": 1720000000000
}
```

Edge Nodeはcollectorと同じSQLite write serializationでrequestを適用します。Exact identity/epoch、未使用publication log/sequenceを確認し、pre-activation `readings.seq`境界を一度だけ固定し、activation receiptを保存してfuture transactionのadmissionを開きます。Measurement、annotation、epoch start、commissioning smoke、quarantine releaseを含む全enqueue pathがこのgateを使います。同じactivation IDのreplayは同じresultを返し、境界を再計算しません。Active Nodeへの別IDは拒否します。

```json
{
  "schema_version": 1,
  "activation_id": "act-0123456789abcdef0123456789abcdef",
  "edge_id": "edge-0123456789abcdef0123456789abcdef",
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "01J...",
  "status": "applied",
  "discard_through_reading_seq": 842,
  "first_publication_seq": 1,
  "applied_at": 1720000001000
}
```

固定prefixは通常query/publication対象外になります。物理削除はrestart可能・boundedなEdge Node-local cleanupで、activation完了条件でも境界変更でもありません。Post-activation outboxのadvance/purge権威は`accepted-through`だけです。

IoTKit Edge stateは`discovered`、`activating`、`active`、`recovery_hold`です。Matching activation resultをcommitして`active`になった後だけrecordを受理します。State、exact epoch、raw insert、fingerprint、cursor advanceは同一custody transactionです。完了前recordは保存せずackも返しません。

## Descriptor snapshot

Descriptor topicはEdge Node所有device/signal metadataのschema version 2 complete snapshotです。他versionを拒否し、schema 1互換pathはありません。MQTT接続ごととpersist済みrevision変更時にpublishし、encoded sizeは1 MiB以下、超過時はtruncateせず拒否します。

任意`model_id`は明示persistしたsoftware catalog IDで、1–64 ASCII byteの`[a-z][a-z0-9]*(?:[-_.][a-z0-9]+)*`です。Display label、device identity、semantic分類ではありません。IoTKit Edgeは表示できますが、semantic mapping、grouping、authorizationを分岐しません。

Snapshotは`system_id`、`series_key`、任意display ID、device state、measurement key、channel、variant、canonical unit、value typeを含みます。Hardware/provider ID、Adapter type/instance、physical locator、configured source、credential、Adapter payloadを含みません。Lower revisionはignoreし、同epoch・同revision・異内容はconflictです。Descriptorはinactive Nodeをdiscoverできますがactivateせず、purge/admission/ackを変更しません。

## Record batch

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "01J...",
  "publication_id": "edge-node-01:01J...:123:130",
  "cursor_start": 123,
  "cursor_end": 130,
  "records": []
}
```

- Rangeはnon-empty・contiguousで、先頭末尾`pub_seq`が一致し、gap/duplicateなし。
- Retryは同じpublication ID、range、record contentを維持。
- Global identityは`(edge_node_id, ledger_epoch, pub_seq)`。
- Event timeは遅延・非単調でもよく、cursorに使わない。
- V1 batchは256 record、encoded 1 MiB以下。
- 初期publisherはapplication未ack batchを同時に一つだけ許可。
- 新active streamは`pub_seq=1`から始まり、架空prefixなし。

`publication_id`は決定的correlation/replay IDです。IoTKit Edgeはrecord content fingerprintを保存します。同一global identityで異なるcontentはcustody conflictで、last-write-wins禁止です。

## Record family

V1は`measurement`、`annotation`、`commissioning_smoke`だけを受理します。Required field欠落、unknown field/enum/familyはraw保存・cursor advance前にbatch全体を拒否します。Field/family追加はversioned contract変更です。

### Measurement

```json
{
  "family": "measurement",
  "schema_version": 1,
  "epoch": "01J...",
  "pub_seq": 123,
  "series_key": "opaque-stable-series-key",
  "values": [21.5],
  "event_time": 1720000000000,
  "event_time_source": "device",
  "time_source": "device_ntp",
  "time_quality": "synced",
  "received_at": 1720000000123,
  "device_time": 1720000000000
}
```

`device_time`は必須fieldですが`null`可能。`values`はnon-empty finite numberです。`time_source`は`device_ntp`、`device_rtc`、`edge_node`、`edge_node_adjusted`、`time_quality`は`synced`、`holdover`、`unsynced`です。`event_time_source`は`device`、`edge_node_adjusted`、`received_at`で、選択timestampと一致しなければなりません。`series_key`はnon-empty opaqueです。

### Annotation

V1は`epoch_start`だけです。`prior_epoch`は必須non-emptyで、measurementと同じpublication sequenceへ参加します。

```json
{"family":"annotation","schema_version":1,"epoch":"01J...","pub_seq":130,"subtype":"epoch_start","prior_epoch":"01H..."}
```

### Commissioning smoke

```json
{"family":"commissioning_smoke","schema_version":1,"epoch":"01J...","pub_seq":131,"test_id":"smoke-0123456789abcdef0123456789abcdef"}
```

任意familyで、物理sensorを装わず通常outbox、MQTT、raw store、`accepted-through`を証明します。`test_id`は`smoke-` + 新規128-bit lowercase hexです。Device registration、registry、quarantine、semantic projectionをbypassしますが、通常sequenceとack contractを使います。

## Application custody acknowledgement

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "01J...",
  "publication_id": "edge-node-01:01J...:123:130",
  "accepted_through": 130
}
```

IoTKit Edgeは一つのcustody transactionで、topic/active state/version/identity/epoch/rangeを認証・検証し、全raw recordをinsertまたはexact replay確認し、contiguous cursorをadvanceし、fingerprintとともに選択正本storeへatomic commitします。そのcommit後だけcorrelated ackをpublishします。

Storage failure、ENOSPC、corruption、commit前cancel、gap、content conflictではackを出しません。Lost ackはexact replayで安全に収束します。Edge Nodeはschema、topic/body identity、epoch、publication ID、monotonicity、batch boundを検証してからcursorを進めます。MQTT PUBACKはcursorもpurge権威も進めません。Rust製Edge Node publisherとRust製IoTKit Edge decoderは`testdata/egress/v1/record-family-cases.json`へ同じaccept/reject結果を返します。

## Retry・停止・認証

- Edge Node outboxはapplication ackまでのretry権威。
- Activation command outboxとEdge Node receiptはactivation retry権威。Broker sessionではない。
- Inactive中もbounded local commissioning collectionを続けるがR10 backlogを作らない。
- IoTKit Edge/network停止中もlocal collectionと未ack rowを保持。
- Reconnect後は同じbatchをcontiguous cursor確認まで再送。
- IoTKit Edge exact replayは既存rowを検証し、commit済みwatermarkを再publish。

初期実装はoperator提供IP path上のMQTT/TLS、anonymous無効、Edge Node別static credential/topic ACLです。Local network、VPN、private route等を使えますが特定VPN製品を要求しません。SecretはGit、argv、log、Debug、audit detail、query outputへ出しません。Plain MQTTは`allow_insecure=true`を明示したlocal Docker testだけです。

## Pinikiet境界と延期項目

R10はcanonical Observationを運びます。IoTKit Edgeが保存seriesを`production`等へ写像しますが、そのmappingはR10へ入りません。Pinikietの業務成功はcustody ackではなく、製品、工程、生産record、OEE、alarm、UI、通知はPinikietが所有します。

暗号化backupを使うoperator承認済みhardware recoveryだけは、[Edge Node復旧契約](edge-node-recovery-v1.md)に従い、旧credential generationをfenceしてsame-IDの新ledger epochへreactivateできます。これは通常のactivation、Edge間transfer、cloneの自動採用ではありません。

このexact recovery case以外のdeactivation/reactivation、IoTKit Edge移管、Edge Node ID再利用、clone検出、standalone outbox自動adopt、same-epoch start boundary、terminal/gap repair、fleet operation、Broker fan-out、legacy HTTPS migration、別egress bindingは延期中です。曖昧なlegacy/restore stateは`recovery_hold`にし、自動activationやremote cleanupを行いません。
