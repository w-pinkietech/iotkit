---
type: Contract
title: "IoTKit Input Adapter host契約 v1"
description: "Input Adapterのidentity、権限、host API、設定、lifecycle、conformanceを定義します。"
language: ja
translation_key: contracts.input-adapter-v1
status: stable
revision: 2
---

# IoTKit northbound Input Adapter host契約 v1

状態: 承認・実装済み（2026-07-20）。

## 1. 範囲

この契約はIoTKit Edge Nodeへcompileする公式in-process sensor Adapterのnorthbound拡張境界です。Edge Node composition rootが選択したvendor Adapter crateをlinkできる一方、汎用host/coreをBravePIから独立させます。

測定契約は既存の`iotkit-ingest-contract::Envelope` / `EnvelopeAck`だけです。別payload、Ack、measurement語彙、device identityを追加しません。完全なD4 Adapterには別のD12 care-servicer channelも必要で、この契約適合だけではcapability declarationやcare-servicer完了を主張しません。

V1はcompile-time catalogです。Installed Adapter type追加にはEdge Node rebuildが必要です。Dynamic library、runtime plugin discovery、ConsoleからのAdapter installは対象外です。

## 2. Identityと権限

| Identity | Owner | 意味 |
|---|---|---|
| `adapter_type_id` | Adapter package/build | Software type。例`bravepi-mainboard` |
| `adapter_instance_id` | Deployment config | 一つのEdge Node上の安定configured instance |
| `configured_source` | Edge Node composition | 診断用Envelope source |
| `principal_id` | Edge Node collector境界 | Receiver所有のadmission/dedup権限 |
| `subject_hint` | Observation | 観測deviceのhardware/protocol identity |
| `system_id` | Edge Node ledger | IoTKit所有の安定device identity |

これらは必ず分離します。Transport pathは設定でありinstance identityではありません。Type/instance IDはrestartやdevice-path aliasをまたいで安定させます。

- Type: 1–63 ASCII byte、`[a-z][a-z0-9]*(?:-[a-z0-9]+)*`
- Instance: 1–63 ASCII byte、`[a-z][a-z0-9]*(?:[-_][a-z0-9]+)*`
- Source: 1–128 ASCII byte、`[A-Za-z0-9][A-Za-z0-9._:/-]*`

Trim、case fold、Unicode normalization、automatic suffixを行いません。Collisionはstartup errorです。公式Adapterは現行`OfficialDiscovery` scopeを維持し、`principal:{configured_source}`を作ります。Source自体は権限を与えません。Edge Nodeがsource-bound facadeを注入し、Adapter codeは`Envelope.source`やprincipal scopeを選べません。

Positional identityでは`subject_namespace`はexact `configured_source`です。Globalに安定したhardware IDを持つAdapterはnamespaceを受け取りますが、`subject_hint`へ含める必要はありません。

## 3. 責務

```text
physical device
  -> transport backend
  -> device driver / codec
  -> shared adapter runtime
  -> adapter-package composition glue
  -> SourceBoundIngest / IngestClient
  -> receiver-owned principal
  -> Edge Node collector / ledger / timeseries
```

Driver/runtimeはingest contract/clientを知りません。Package composition glueがhost APIを所有します。Adapter crateはEdge Node SQLiteへアクセスせず、collector、ledger、registry、timeseries、publish、ops、engine、IoTKit Edgeへ直接依存しません。

Edge Nodeはprincipal作成、設定認可、inventory mutation、start/stop/restart、backoff、exhaustion、health集約、static type catalogを所有します。同じprincipal-bound clientはAdapter restartをまたいで維持します。

## 4. Host API

`iotkit-input-adapter-host-api`はsupervision非依存の次のprimitiveを提供します。

```text
AdapterStartContext {
  instance_id,
  configured_source,
  subject_namespace,
  ingest: SourceBoundIngest,
}

SourceBoundIngest::try_submit(items)
  -> EnqueuedEnvelope { envelope_id, delivery: DeliveryReceipt }
  | QueueSubmitError::{Full(RetryHandle), Closed(RetryHandle)}

SourceBoundIngest::try_retry(RetryHandle)
  -> EnqueuedEnvelope
  | RetryQueueError::{Full(RetryHandle), Closed(RetryHandle), SourceMismatch(RetryHandle)}

DeliveryOutcome {
  Final(EnvelopeAck),
  AbandonedBeforeFinal {
    reason: SpoolOverflow | ClientShutdown | CollectorClosed,
    retry: RetryHandle,
  },
}

RunningInputAdapter {
  instance_id, activity, diagnostics, completion, shutdown,
}
```

Facadeはbound source、既存ID recipe、`declaration_version=None`でimmutable Envelopeを作ります。Queue admissionはnon-durableでcustodyを移しません。Receipt/retry handleは`iotkit-ingest-client`所有で、host APIはwrap/re-exportだけします。

保持されたreceiptはexact一回resolveします。Final `Accepted`、`Duplicate`、terminal `Rejected`はexact Ackを返し、`Deferred`とno-ackはpendingです。Spool evictionやclient/collector終了は、同じimmutable Envelope/IDをopaque retry handleに入れた`AbandonedBeforeFinal`を返します。

`try_retry`はbound sourceを検証してunchanged Envelopeを再queueします。`Full`はhandle ownershipをcallerへ返します。Packageはfinal Ack semanticsだけでupstream cursorを進め、local abandonmentからcustodyを捏造しません。

Activityはsuccessful physical decode、queue admissionのprocess-monotonic timestampとdropped diagnostic countを持つcoalescing latest snapshotです。Diagnosticはbounded、best-effort、redactedで、generic kindは`Transport`、`Protocol`、`Decode`、`MeasurementMapping`、`ClientQueueFull`、`ClientClosed`、`DeviceUnavailable`です。Adapter固有codeはtype ID namespaceを使い、diagnosticをauthoritative healthとしません。

Completionは全async taskとblocking reader thread停止後にlossless resolveし、`RequestedStop`、reason付き`UnexpectedExit`、`Panic`を表します。Shutdownはidempotent graceful stop requestだけで、timeoutはEdge Nodeが所有します。Start error後にtask/thread/open transportを残しません。

Host APIは`edge-node/core/supervision`へ依存せず、`AdapterEvent`/`AdapterCommand`、principal作成、storage、設定認可、restart policy、health assertionを公開しません。

## 5. Type catalogと設定

Built-inは非secret `InputAdapterTypeDescriptor`を返します。

- `adapter_type_id`
- `adapter_api_major`
- `config_schema_version`
- 診断専用`implementation_version`
- `display_name`
- `physical_transport_kind`

Ingest contract、Adapter API、config、implementation、device `declaration_version`は別version domainです。Factoryは`iotkit-edge-node` privateで、`descriptor()`、`parse_and_validate(raw_config)`、`start(edge_context, validated_config)`だけを公開します。Unknown field/versionを厳格拒否し、全instance、identity collision、source binding、inventory intentをstart前に検証します。

```toml
[adapters.instances.bravepi_main]
type = "bravepi-mainboard"
enabled = true
config_schema_version = 1
source = "input:bravepi-mainboard:bravepi_main"
port = "/dev/serial0"

[adapters.instances.local_i2c]
type = "rpi-local"
enabled = true
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "mcp9600"
address = 0x60
thermocouple_type = "K"

[[adapters.instances.local_i2c.devices]]
model = "opt3001"
address = 0x44
```

Table keyがstable instance IDで、`source`は必須・安定です。Principal IDとpositional namespaceはsourceから導出します。`rpi-local`の`model`はAdapter packageのcompile-time catalogが解決し、Edge Nodeはmodel IDで分岐しません。MCP9600の`thermocouple_type`は`K/J/T/N/S/E/B/R`、OPT3001はmodel固有設定なしです。Empty list、unsupported model/setting、invalid/duplicate I2C address、polling interval不整合は一つもstartする前に失敗します。

`devices`省略時はMCP9600 `0x60` K-typeとOPT3001 `0x44`の互換inventoryです。Entry削除・instance disableはrestart後のpollingを止めますが、ledger device/seriesを黙ってretireしません。物理撤去・交換はledger retire/replacement operationを使います。Pi 4B/5を設定で選ばず、transport backendが必要Linux I2C能力を検査します。

Legacy formとinstance formは排他です。Absent configはBravePI `/dev/ttyAMA0`を有効、RPi-localを無効にします。Legacy overrideはlegacy formだけへ適用します。Source/subject recipe変更には既存DBを使うcutover testが必要です。

## 6. Measurement、descriptor、inventory

Driver/protocolから物理値への変換と、Adapter packageからcanonical measurementへのprojectionはAdapter所有です。Shared runtimeはdevice model catalogやmeasurement keyを知りません。Conformance fixtureがdriver値・unit、finite transform、canonical key/UCUM、value count、channel、variantを固定し、registryとexact emitted itemに照合します。

Supported mappingはconnected-device capabilityではありません。Retained descriptorは実ledger device、materialized non-quarantined series、provider-neutral registryから作ります。Full capability declaration、care verb、`declaration_version` mismatch、redescribeは別state machineです。

Positional inventoryはEdge Node所有mutationです。全instanceをpure validationし、intentをまとめ、audited typed operationでidempotent reconcileし、collector cache/generationを更新し、commit後にAdapterをstartします。同じresolved target listをreconciliationとruntime startに使い、factory/Adapterはledger/registryを変更しません。

Persisted model IDはdescriptor schema 2へ出す唯一のAdapter-origin metadataで、任意・opaque・display-onlyです。Adapter type/instance、source、bus path、address等はEdge Node-localです。同じsource/locatorへ別modelを割り当てる場合、全reconciliationをstart前に拒否し、明示replacement/cutoverを要求します。

## 7. Lifecycleとlegacy分離

Initial startはfail-fastです。一つが失敗するとstarted instanceを逆順停止してnon-zero exitします。成功後のunexpected exit/restart failureはEdge Node所有のbounded backoff/budgetを使い、exhaustionはprocess-lifetime degraded healthです。Systemd restartでresetします。

Layer checkは全Input Adapterのtransitive Cargo reachabilityを検査し、legacy care pathの`bravepi-mainboard-adapter`だけが`edge-node/core/supervision`へ到達できます。Polling runtimeはsupervision-freeで、decoded Observation/lifecycle factだけを出します。BravePIもnorthbound seamでdecodeとmapping/submissionを分離し、legacy southbound語彙はpackage-private wrapperへ閉じ込めます。

## 8. Conformance

Dev-only testkitは次を再利用可能に検査します。

- Identifier/config validationとcatalog uniqueness
- Source binding、subject stability、registry mapping、unit、channel、finite item
- Generic lifecycle、shutdown、activity、bounded diagnostic
- Spool saturation、receipt resolve、unchanged retry、final Ack、close
- Multiple same-type instance、legacy identity、inventory/runtime parity、reverse shutdown、generation fence
- Panic/stop cleanup、transitive negative layer fixture

第三Adapter追加で変えるのは、そのcrate、Edge Node-private catalog entry、Cargo/layer分類、architecture map、fixtureだけです。Collector、storage、MQTT custody、IoTKit Edge、semantic、Output Adapterは変更しません。Test-only reference Adapterは2 subject・2 measurementをBravePI typeなしで出しますが配布しません。

## 9. 対象外

- Runtime pluginまたはlanguage-neutral in-process ABI
- Third-party Adapterの自動trust/code signing
- Full capability/redescribe convergence
- Generic care-command redesign
- External Connector、camera、barcode、actuator
