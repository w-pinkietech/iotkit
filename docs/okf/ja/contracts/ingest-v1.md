---
type: Contract
title: "IoTKit認証付きingest契約 v1"
description: "認証付きHTTP ingestのwire schema、権限、retry、validation、有限上限、復旧semanticsを定義します。"
language: ja
translation_key: contracts.ingest-v1
status: stable
revision: 3
---

# IoTKit認証付きingest契約 v1

状態: **認証付きHTTP device-ingest bindingの規範文書**。

この契約は`iotkit-ingest-http`が実装します。`iotkit-ingest-contract`のJSON typeは配布するreference表現であり、この文書がdevice開発者向け契約です。Endpointは`/api/v1`でversion管理し、認証付きlocal-network listener上のJSONを受理します。

## 最初の3 command

Device開発者が接続する前に、operatorは次をhandoffします。

- `https://` schemeとportを含む有効なEdge Node URL
- credential operationが一度だけ表示する現在のdevice bearer token
- 選択したingress Edge Node public certificateを保存したPEM CA/trust-anchor file
- tokenへ設定したsource identifier（通常は安定した`principal_id`）

Operatorはone-time tokenに`iotkit-edge-nodectl device-credential issue`を使えます。Construction-tier ingress TLS operationが返すfingerprintと、選択された`ingress-tls/generation-N`のcertificateが一致しなければなりません。Control-plane certificateを表示する`iotkit-edge-nodectl fingerprint`は、意図的に同じcertificateを両listenerへ設定した場合を除き、ingress trust anchorではありません。Bearerをsource controlや共有imageへ入れてはいけません。

```sh
export IOTKIT_URL="${IOTKIT_OPERATOR_URL:?set by operator handoff}" IOTKIT_TOKEN="${IOTKIT_OPERATOR_TOKEN:?set by operator handoff}" IOTKIT_CA="${IOTKIT_OPERATOR_CA:?set by operator handoff}" IOTKIT_SOURCE="${IOTKIT_OPERATOR_SOURCE:?set by operator handoff}" IOTKIT_ENVELOPE="${PWD}/one-envelope.json"
printf '%s\n' '{"envelope_id":"builder-example-0001","source":"'"$IOTKIT_SOURCE"'","items":[{"measurement_key":"temperature_c","values":[21.5],"time_source":"edge_node"}]}' > "$IOTKIT_ENVELOPE"
curl --fail-with-body --silent --show-error --cacert "$IOTKIT_CA" --header "Authorization: Bearer $IOTKIT_TOKEN" --header "Content-Type: application/json" --data-binary "@$IOTKIT_ENVELOPE" "$IOTKIT_URL/api/v1/ingest"
```

`--cacert`を`--insecure`へ置き換えてはいけません。Retryのたびに同じimmutable Envelope、同じ`envelope_id`、同一payloadを保持します。送信側がlocal copyを捨てられるのは`200` acknowledgementだけであり、捨ててよいかはack statusで判断します。

ESP32も同じHTTPS endpoint、bearer header、JSON Envelope、retry ruleを使います。Edge Node certificateまたは承認済みpublic SPKI trust anchorをread-only trust storeへprovisionし、毎回certificateとhostnameを検証します。Server認証を伴わないbearer tokenは対応構成ではありません。

## Wire schema

`POST /api/v1/ingest`は`Content-Type: application/json`で、一つの`Envelope`を送ります。

```json
{
  "envelope_id": "builder-example-0001",
  "source": "<operator-provided principal_id>",
  "items": [
    {
      "measurement_key": "temperature_c",
      "values": [21.5],
      "time_source": "edge_node"
    }
  ]
}
```

### Envelope field

「独立field上限なし」はRust typeが追加の長さ・非empty検査をしないという意味です。HTTP decoderの有限body上限は常に適用されます。Integer範囲はRust表現のexact範囲です。

| Field | JSON type | 必須 | 制約と受信側の意味 |
|---|---|---|---|
| `envelope_id` | string | 必須 | 独立上限なし。Retry間でbyte単位に維持し、dedup scopeは認証principal |
| `source` | string | 必須 | 独立上限なし。診断・設定値であり、権限selectorではない |
| `declaration_version` | unsigned integer (`u32`) / `null` | 任意 | `0..=4,294,967,295`。現行collectorは保持するが宣言依存behaviorなし |
| `items` | `ReadingItem` array | 必須 | 配布collectorは`0..=256`。HTTP deploymentはより小さい正の上限を設定可能 |

Empty `items`は表現可能です。`envelope_id`と`source`をdefault・推測しません。

### ReadingItem field

| Field | JSON type | 必須 | 制約と受信側の意味 |
|---|---|---|---|
| `subject_hint` | string / `null` | 任意 | 独立上限なし。One-subject認証scopeだけ省略可能 |
| `measurement_key` | string | 必須 | `.`区切りの1つ以上の`[a-z][a-z0-9_]*` segment、UTF-8で64 byte以下 |
| `channel_index` | unsigned integer (`u16`) | 任意 | `0..=65535`。省略時はdeclarationのno-channel sentinel |
| `series_variant` | string / `null` | 任意 | 独立上限なし。省略時のreceiver defaultは`primary` |
| `values` | finite number (`f64`) array | 必須 | 全値finite。Registry declarationがcount/typeを追加制約可能 |
| `device_time_ms` | signed integer (`i64`) | 任意 | Unix msの`i64`全域。使用時はabsolute freshnessを適用 |
| `time_source` | string enum | 必須 | `device_ntp`、`device_rtc`、`edge_node`、`edge_node_adjusted`だけ |
| `age_ms` | unsigned integer (`u64`) | 任意 | `u64`全域。受理時はfreshness window内かつreceiver subtraction可能 |
| `rssi` | signed integer (`i16`) | 任意 | `-32768..=32767`、任意radio metadata |
| `battery_pct` | unsigned integer (`u8`) | 任意 | `0..=255`。型は別途`0..=100`検査を追加しない |

`time_source: edge_node`でabsolute device timeがなければEdge Node receive timeを使います。有効な`age_ms`はreceive timeから差し引き、`edge_node_adjusted`として記録します。両方ある場合は`device_time_ms`を優先します。

## Acknowledgement

Commit済みrequestは`EnvelopeAck`を返します。

```json
{
  "envelope_id": "builder-example-0001",
  "status": {
    "kind": "accepted",
    "items": [{ "kind": "stored", "disposition": "durable" }]
  }
}
```

Accepted item statusはrequest itemと同じ長さ・順序です。Stored itemの`disposition`は`durable`、`staged`、`quarantined`で、quarantine時は`quarantine_reason`を持てます。Terminalな`item_rejected`は`reason_code`、`message`、任意のJSON Pointer `field_path`と`schema_hint`を持ちます。

| Field / variant | JSON type | 制約 |
|---|---|---|
| `EnvelopeAck.envelope_id` | string | 提出identifierをecho |
| `EnvelopeAck.status` | tag付きobject | tagは`kind`、下記statusのexact一つ |
| `accepted.items` | `ItemStatus` array | Request itemとexact同数・同順 |
| `duplicate` | 追加fieldなし | `{"kind":"duplicate"}`。同一spool copyを破棄可能 |
| `rejected` | reason/message/任意path/hint | 決定的でterminalなEnvelope違反 |
| `deferred` | 追加fieldなし | 内部契約語彙。通常HTTP admissionはackなし`429`/`503` |
| `stored.disposition` | enum | `durable`、`staged`、`quarantined` |
| `stored.quarantine_reason` | enum / `null` | Concrete reasonがあるquarantine rowに付与 |
| `item_rejected` | reason/message/任意path/hint | 決定的でterminalなitem違反 |

### ValidationReport

`POST /api/v1/ingest/validate`は`EnvelopeAck`ではなく次を返します。

| Field | JSON type | 制約 |
|---|---|---|
| `envelope_id` | string | Submitted IDをecho |
| `valid` | boolean | `issues`がemptyのときだけ`true` |
| `issues` | `ValidationIssue` array | 決定的かつ副作用なし |
| `item_index` | non-negative integer / `null` | Envelope-wideなら省略、itemは0-based |
| `reason_code` | enum string | Ack診断と同じ安定語彙 |
| `message` | string | 人間向けだけ |
| `field_path` / `schema_hint` | string / `null` | JSON Pointerと安定hint |

### 安定enum語彙

| Enum | Value |
|---|---|
| `AckStatus.kind` | `accepted`, `duplicate`, `rejected`, `deferred` |
| `ItemStatus.kind` | `stored`, `item_rejected` |
| `Disposition` | `durable`, `staged`, `quarantined` |
| `QuarantineReason` | `out_of_range`, `unknown_key`, `undeclared_channel`, `device_quarantined` |
| `TimeSource` | `device_ntp`, `device_rtc`, `edge_node`, `edge_node_adjusted` |
| `ReasonCode` | `malformed_measurement_key`, `value_type_mismatch`, `unknown_subject`, `subject_scope_violation`, `batch_too_large`, `stale_timestamp`, `internal` |

`internal`はread-compatible legacy値で、現行producerは出力しません。Storage/commit failureを`rejected`で表現せず、ackなし`503`またはconnection failureにします。

Accepted ackは、readingとsame-transaction publication record、またはdispositionが示すbounded staging/dedup stateがEdge Nodeのdurability pointへ達したことを意味します。Duplicateはdedup window内に元Envelope claimがあり、sender copyを破棄できることを意味します。どちらもarchive consumerのcustody取得を保証しません。

## 認証・subject・権限

`Authorization: Bearer <device-token>`を送ります。Tokenはopaqueで、at-rest hash、plaintext非公開比較を使い、log、error、audit detail、health、fixture、`Debug`へ含めません。Missing、invalid、revoked、stale tokenはingest ackなし`401`です。

認証principalがsubject scope、dedup namespace、flow accounting、audit attributionを所有します。`Envelope.source`は選択できません。

- One-subject tokenは`subject_hint`を省略可能。
- Multi-subject tokenは全itemで`subject_hint`必須。
- Scope外subjectはitem-level `subject_scope_violation`。
- HTTP tokenが指定するunknown subjectはitem-level `unknown_subject`で、network pathではstagingしない。
- Bounded unknown-subject sightingを作れるのはtrusted official in-process Adapter principalだけ。
- Source/principal mismatchはEnvelope-level terminal `rejected`で、bounded intrusion signalを出し、権限を広げない。

Item failureは位置対応で、valid siblingは同じaccepted Envelopeでcommitできます。

## 時刻とfreshness

Receiverがreceive timeとclock provenanceを所有します。Default absolute freshnessは24時間、future skew allowanceは5分で、deploymentは実装範囲内のより小さい有限値を選べます。

| 入力とEdge Node clock | 結果 |
|---|---|
| `device_time_ms`なし、`edge_node` | Receive timeで受理 |
| Absolute timeなし、有効な`age_ms` | Receive time minus age、`edge_node_adjusted`で受理 |
| `age_ms`がwindow外または減算不能 | Terminal item rejection |
| Trusted clockとfreshな`device_time_ms` | Wall clock比較後に受理または`stale_timestamp` |
| Clock untrustedで`device_time_ms`あり | Envelope全体ackなし`503`、clock復旧後に同一retry |
| 過去window外 | 位置対応`stale_timestamp`、valid siblingはcommit可能 |
| Future allowance超過 | Terminal freshness rejection、device clock修正 |

Startup時はclock-untrustedです。Sync evidenceまたはlocal-root確認が必要です。Backward stepやauth-time-floor write失敗はfail closedします。Restart時はnondecreasing floorをreloadしますが、clock trustは再びuntrustedから始めます。

## RetryとHTTP response

Senderはretryごとにexact Envelopeを保持し、bounded exponential backoffとjitterを使い、`Retry-After`を守ります。Custodyをclaimしないresponseでsender copyを削除しません。

| 条件 | HTTP response | Sender action |
|---|---|---|
| Commit済みaccepted/duplicate/terminal result | `200` + `EnvelopeAck` | Statusに従う。Accepted/duplicateだけ破棄可能 |
| Credential missing/invalid | `401`, ackなし | Credential修復、custodyを仮定しない |
| Source/principal mismatch | `200` + envelope `rejected` | Source修正、unchanged Envelopeはterminal |
| Item subject failure | `200` + positional `item_rejected` | Valid sibling resultを保持し将来入力を修正 |
| Throttle | `429` + bounded `Retry-After`, ackなし | 同一Envelopeをbackoff+jitterでretry |
| Queue unavailable / draining | `503`, ackなし | 同一Envelopeをretry |
| Storage/commit/clock/internal failure | `503`またはconnection failure、ackなし | 保持して同一retry |
| 安全にparseできた決定的malformed/oversize | `200 rejected`またはbounded `4xx` | 入力修正、blind retryしない |

`429`、`503`、timeout、connection close、empty bodyは暗黙の`rejected`ではなく、spool削除を許可しません。

## 副作用のないvalidation

`POST /api/v1/ingest/validate`はingestと同じbearer認証、exposure、header/body/time上限、principal scopeを使います。Parsing、source/scope、schema、subject、freshnessを検査しますが、reading、dedup claim、staging row、custody state、ingest ackを書きません。Security違反はbounded intrusion episodeを作る場合があります。Validation成功だけでspoolを削除せず、同一Envelopeを`/api/v1/ingest`へ提出します。

## 有限上限とlistener exposure

Listenerは既定無効で、control APIとは別のconstruction-tier local-network listenerです。TLSが通常modeです。Private-LAN plaintextは明示的なdegraded modeで、上記journeyには使いません。Wildcard、public、Internet-capable、proxy-derived exposureを拒否し、peerは設定済みprivate local ingress CIDR内に限定します。

| Resource | Default |
|---|---:|
| measurement key | UTF-8 64 byte、`.`区切り`[a-z][a-z0-9_]*` |
| request header | 32 / 8,192 byte |
| decoded JSON body | 65,536 byte |
| items / envelope | hard 256 / HTTP default 256 |
| concurrent request / connection | 16 / 32 |
| collector queue | 8 |
| auth cache | 64 entry / 60 s |
| header / request / collector wait | 5 s / 10 s / 5 s |
| TLS handshake | 5 s / peer |
| Retry-After | 1 s、設定範囲`1..=3600` |
| pre-auth state | 1,024 entry / 60 s / window内8 failure |
| auth worker / reserved | 2 / 1 |
| general auth rate / burst / initial | 16 / 32 / 1 |
| reserved auth rate / burst / initial | 8 / 8 / 1 |
| principal capacity | 64 |
| principal low/default/high | 各1,000,000 rate/burst unit |
| global flow rate / burst | 4,000,000 / 4,000,000 |
| throttle cooldown | 5,000 ms |
| freshness / future skew | 86,400,000 / 300,000 ms |
| dedup row / principal row / age | 100,000 / 10,000 / 72 h |
| unknown staging global | 10,000 row / 64 MiB / 30 day |
| unknown staging per principal | 1,000 row / 8 MiB |
| staging reserve | 256 row / 64 KiB |
| staged row / hardware ID | 1,000 |
| dedup purge interval | 3,600,000 ms |
| TCP backlog | 128 |

Stagingは最大Envelope一件分のevictable reserveを持ち、rowとbyteを計上します。Dedup keyは`(stable_principal_id, envelope_id)`で、ageとrow上限を持ちます。Maintenance purge失敗はhealthとduplicate suppression保証を劣化させますが、commit済みackを未ackへ変更しません。Admission reservationはtimeout、disconnect、parse failure、cancel、queue failure時に解放します。Invalid inputでcache、queue、source map、auditを無制限に増やしません。

## 現行実装・延期項目・復旧

現行実装は、認証付きHTTP binding、bearer authority、bounded admission、TLS/private-LAN listener、freshness、副作用なしvalidation、bounded staging/dedup、health/audit hook、local recovery authority closure、custody-completeなsanitized Edge Node DBの暗号化backup、local fenced-candidate restoreを提供します。Restore境界はclosedなvalid handoffを要求し、absent candidateだけへ書き、candidateをfencedのまま残します。RemoteからclaimできるEdge Node setup pathではありません。

Broker fencing、remote permit、production/remoteでのrename後reconciliation、dedup-risk resolution、reactivation、same-ID new ledger epochは延期かつdefault-offです。Completed candidate rename後のexact local same-request replayは出荷済みで、保存済みreceiptをbyte-for-byte返しますが、production/remote reconciliationではありません。Slice 1にはproduction recovery handoffのproducerがなく、後続のpermitとcredential-generation checkまでcandidateはingestもpublishもできません。Device-token secretが存在する場合、legacy plaintext replacement snapshotは利用できません。Tokenやhashを出力せず、その理由を示します。State-only inspectionはcompleteなreplacement backupではありません。

Snapshot sanitizerは`target_registry`のdeployment credential tokenを空にします。Account、session、device credential hashはprotectedな暗号化DB stateとして残り得ます。MQTT/TLS private materialはそのDBの外にありartifactへ入れません。暗号化artifactとpassphraseはsecretです。

MQTT ingest、pairing-window登録、`signed_seq`、`provisioned_key`、batch provisioning、shared-image credential、rich UI、destructive factory resetは将来または別承認の範囲で、このHTTP token/TLS/subject/freshness/custody契約を迂回しません。

Unowned、local recovery中、restore/reset fence中、TLS invalidなEdge Nodeはcontrol APIもingest listenerもbindしません。Local root recoveryでownershipを再確立し、restore後はadmin/operator/session権限を失効し、device auth generationを再検査します。暗号化backup candidateはauthenticated snapshot boundaryまでのreadingとdedup claimを持ちますが、fenced中はingestできず、その後のretry stateを証明しません。後続のpermitとgeneration checkまでrestore済みdedupをactiveにしません。Backupなしのreplacementはreadingもdedup claimもrestoreしません。Downstream idempotencyと後続の新ledger epochが重複可能性を表します。

Credential responseはone-shotです。失った場合は対象credentialをabandon/revokeして新しくissueし、再表示を要求しません。Deviceを沈黙させ得るissue、reissue、promotion、abandonment、revokeには人間承認が必要です。
