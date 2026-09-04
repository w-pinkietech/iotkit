---
type: Contract
title: "IoTKit Edge Node復旧契約 v1"
description: "Sanitized暗号化Edge Node backup container、fenced-candidate restore、許可されたproduction reactivationを定義します。"
language: ja
translation_key: contracts.edge-node-recovery-v1
status: stable
revision: 5
---

# IoTKit Edge Node復旧契約 v1

Status: **任意のlocal Edge Node backup、fenced-candidate restore、permit済み本番復帰に
対する規範契約**。

この契約は暗号化Node artifactの正確な形式とlocal restore境界のauthorityです。
下記のschema、fixture、conformance testと対になっています。文書、schema、
fixture、exportされたRust typeのどれも他を黙って上書きしません。不一致は
contract defectです。

## 1. 範囲と出荷済み境界

Slice 1はlocal-root `iotkit-edge-nodectl backup` surfaceを提供し、sanitized
Edge Node SQLite DBのcustody-complete暗号化backupをcreate、inspect、status
取得できます。またschema-valid recovery handoffを受けて新しいlocal
candidateをinstallするrestore operationを提供します。Candidateはpublication
前にdurably fencedになります。

Backup configurationとenabled timerは既定ではありません。任意のsystemd
templateはoperatorがinstallしてtimerを明示的にenableするまで不活性です。
Serviceは
`deploy/systemd/iotkit-edge-node-backup.service`、
timerは
`deploy/systemd/iotkit-edge-node-backup.timer`
にあるexact CLIを実行します。

Revision 2はproduction recovery-handoff作成、同梱Broker credential fencing、
candidate-bound permit、accepted-through reconciliation、same-ID new ledger epochでの
reactivationを追加します。Candidateはmatching completionをdurably保存するまで
collect、publish、ingest listener bindをできません。

## 2. Machine authorityとconformance material

対になったmachine artifactがwire authorityです。

| Material | 規範artifact |
| --- | --- |
| Container header schema | `edge-node/core/recovery/contracts/node-backup-header-v1.schema.json` |
| Sanitized manifest schema | `edge-node/core/recovery/contracts/node-backup-manifest-v1.schema.json` |
| Recovery handoff schema | `edge-node/core/recovery/contracts/recovery-handoff-v1.schema.json` |
| Broker fence receipt schema | `edge-node/core/recovery/contracts/broker-fence-receipt-v1.schema.json` |
| Fenced restore receipt schema | `edge-node/core/recovery/contracts/restore-receipt-v2.schema.json` |
| Header golden | `edge-node/core/recovery/tests/fixtures/node-backup-header-v1.json` |
| Manifest golden | `edge-node/core/recovery/tests/fixtures/node-backup-manifest-v1.json` |
| Handoff golden | `edge-node/core/recovery/tests/fixtures/recovery-handoff-v1.json` |
| Broker fence receipt golden | `edge-node/core/recovery/tests/fixtures/broker-fence-receipt-v1.json` |
| Receipt golden | `edge-node/core/recovery/tests/fixtures/restore-receipt-v2.json` |
| Recovery MQTT control golden | `testdata/egress/v1/recovery-activation-{request,result}.json`、`testdata/egress/v1/recovery-completion{,-ack}.json` |
| Binary conformance vector | `edge-node/core/recovery/tests/fixtures/node-backup-v1.bin` |
| Container conformance tests | `edge-node/core/recovery/tests/backup_contract.rs` と `edge-node/core/recovery/tests/unit/container_tests.rs` |
| Restore conformance tests | `edge-node/core/recovery/tests/unit/restore_tests.rs` と `edge-node/apps/node/tests/recovery_startup.rs` |

Checked-in binaryはpublic format vectorだけです。Production encryptionはOS
randomnessを使い、passphrase、key、path、identity、digestをlog、error、
audit record、status output、debug representationへコピーしません。

## 3. Configuration、destination capability、scheduling

`backup configure`はschema-1 owner-only configurationとsystemd drop-inを一つの
guarded publicationとして書きます。Configuration、passphrase、handoff fileは
invoking accountがownerのregular fileで、linkは一つ、group/other permission bit
なし（通常mode `0600`）でなければなりません。既存configurationには明示的な
`--replace-existing` policyが必要です。

Destinationはcapability probeが成功した場合だけsupportedです。Probeは
symlinkをfollowせず対象directoryをopenしてholdし、owner-only modeとwritable
capacity、no-replace publication、file sync、parent-directory sync、
descriptor-relative read-backを検査し、databaseとdestinationのfilesystem境界を
再検査します。Filesystem name、label、mutableな`/dev/sdX`表記はendorsement
ではありません。Persistするmount identityはstable block UUID
（`uuid:<value>`）またはfilesystem IDとdecoded source
（`fsid:<value>|<source>`）で、stable identityがなければfail closedです。
Destinationはlive databaseと異なるfilesystemでなければなりません。

Staging capabilityにはeuid所有でgroup/other writableでない既存`tmpfs` parentと
owner-only leafがあります。Systemd pathでは`/run`が既存tmpfs parentです（通常の
mode `0755`は可）。`configure`はfinal componentをfollowせずparentをopenし、typeと
link countを検証して正確なstaging path `/run/iotkit-edge-node-backup`を記録します。
`/dev/shm` rootのようなworld-writable tmpfsはparentとして受け付けません。`create`は
held parent descriptorからabsent exact leafをmode `0700`で作るか、既存leafのowner-only
type、link count、tmpfsを検証します。任意の`/run` treeを作成・拡張せず、置換pathを
followせず、destinationをstagingに使いません。Restart時にはtmpfs staging内容が消えます。

Configured pathを確認してから任意templateをinstallします。

```sh
sudo install -D -m 0644 deploy/systemd/iotkit-edge-node-backup.service \
  /etc/systemd/system/iotkit-edge-node-backup.service
sudo install -D -m 0644 deploy/systemd/iotkit-edge-node-backup.timer \
  /etc/systemd/system/iotkit-edge-node-backup.timer
sudo install -d -m 0755 /etc/systemd/system/iotkit-edge-node-backup.service.d
sudo systemctl daemon-reload
```

CLIが生成するdrop-inはcapability checkでcapturedしたmount pointだけを使います。
手編集やmount nameの置換はしません。

```ini
[Unit]
RequiresMountsFor=/absolute/captured/mount/point
```

Exact fileを
`/etc/systemd/system/iotkit-edge-node-backup.service.d/`に置いた後、operatorは
明示的にopt-inできます。

```sh
sudo systemctl enable --now iotkit-edge-node-backup.timer
sudo systemctl status iotkit-edge-node-backup.timer
```

`enable --now`がactivation decisionです。Installとdaemon reloadだけではtimerは
enableされません。Configuration後のmanual one-shot checkは
`systemctl start iotkit-edge-node-backup.service`です。Create失敗はaccepted
backupではなく、live database削除を許可しません。

## 4. 暗号化container framing

Artifactは8 byte ASCII magic `IOTKNDB1`で始まります。Edge server backupの
magic、unknown artifact kind、unknown version、unknown algorithmはrejectします。
Magicの後に4 byte big-endian header lengthと、その長さのexact header JSON bytes
が続きます。Header lengthは0でなく16 KiB以下です。Header JSONはclosed
（`additionalProperties: false`）で、magic、length、exact JSONの全byteを
authenticateします。

Header fieldとboundは次のとおりです。

| Field | Exact valueまたはbound |
| --- | --- |
| `artifact_kind` | `iotkit_edge_node_database` |
| `format_version` | integer `1` |
| `kdf` | `argon2id` |
| `salt_b64` | exactly 16 bytesのcanonical unpadded Base64（22文字） |
| `kdf_time` | integer `1..=10` |
| `kdf_memory_kib` | integer `16,384..=262,144` |
| `kdf_parallelism` | integer `1..=16` |
| `cipher` | `xchacha20-poly1305` |
| `nonce_prefix_b64` | exactly 16 bytesのcanonical unpadded Base64（22文字） |
| `chunk_size` | integer `4,096..=4,194,304` bytes |

新規artifactを作るv1 writerの既定値は固定です。

| Writer field | v1 default |
| --- | ---: |
| `kdf_time` | `3` |
| `kdf_memory_kib` | `65,536` KiB |
| `kdf_parallelism` | `4` |
| `chunk_size` | `262,144` bytes |

Readerは上記bound内の任意の値を受け付けなければなりません。Writer defaultは
readerの追加制限ではありません。

Keyはowner-supplied passphrase、authenticated salt/parameterからArgon2id
(version 1.3)で32 byte導出します。Saltとnonce prefixはOS randomnessです。
Passphraseとderived keyはzeroizeします。Deterministic entropyはtest codeだけに
存在しproduction codeから呼べません。

Header後の各recordは次です。

```text
flags:u8 || plaintext_length:u32be || ciphertext_and_tag
```

Data recordは`flags=0`、plaintext lengthは0より大きくheader `chunk_size`以下、
Poly1305 tagは16 byteです。Terminal recordはちょうど一つ、`flags=1`、length 0
です。Terminalはauthenticateし、manifestとdatabase bytesの後で、immediate EOF
が続かなければなりません。Unknown flag、truncation、duplicate/early terminal、
zero-length data、oversized chunk、sequence overflow、malformed length、trailing
byteはinvalidです。

Record sequenceは0から開始します。XChaCha20 nonceは16 byte nonce prefixに
unsigned 64-bit big-endian sequenceを続けたものです。Associated dataは正確に
`header_digest || sequence:u64be || flags:u8 || plaintext_length:u32be`で、
`header_digest`は
`SHA-256(MAGIC || header_length:u32be || exact_header_json)`です。

Authenticated plaintext streamは次です。

```text
manifest_length:u32be || manifest_json || sanitized_sqlite_bytes
```

Manifest lengthはallocation前に0より大きく1 MiB以下でなければなりません。
Manifest JSONをclosed validationしてからdatabase bytesを受け付けます。
Database lengthとlowercase SHA-256 digestはstream中に計算し、authenticated
manifestと完全一致してからterminalを受け付けます。Authenticationはplaintext
を作成しません。Decryptionはanonymous owner-only staging fileへだけ書き、
existing pathをoverwriteしません。

## 5. Sanitized manifestとdatabase invariant

Manifestは`artifact_kind=iotkit-node-backup`、`format_version=1`、
`snapshot_mode=online`、`shutdown_seal_id=null`、backup作成時のEdge Node schema
versionです。Revision 2はchecked-in version `23` vectorとcurrent version `24`を
受け付け、version `23` databaseをcanonicalに検証してから、activation前にfenced
candidate内でversion `24`へmigrateします。新しいbackupはcurrent schemaを使います。
`backup_id`、`edge_node_id`、
`ledger_epoch`はnonempty、最大255 Unicode scalar、colon/control characterなし、
pathnameから推測しません。Timestamp、cursor、allocation high-waterは
nonnegative signed 64-bit integerで、`accepted_cursor`は
`allocation_high_water`を超えてはなりません。`database_length`と全countは
unsigned 64-bit integerです。`database_sha256`は64個のlowercase hexadecimal
characterです。Closed count fieldは`devices`、`series`、`readings`、
`publication_rows`、`ingest_dedup_rows`、`staged_readings`、
`quarantine_rows`、`device_principals`、`device_credentials`、
`activation_rows`、`ledger_events`、`audit_events`の12個です。
新writerは存在する場合にoptional `epoch_start_publication_seq`も出力します。値は
`allocation_high_water`以下の正確なpositive sequenceです。Format v1互換性のため
optionalのままです。Fieldが欠落していれば、production inspectはauthenticated DBを
anonymous owner-only tmpfs fileへだけdecryptし、recovery authorityのprepare前にDBから
値を導出します。

Sourceはrecovery snapshot operationでcopyし、そのcopyをsanitizeします。
`target_registry.credential_token`を空にし、journal modeをDELETE、secure deletionを
有効化、copyをvacuumし、`-wal`、`-shm`、`-journal` sidecarを残しません。
Canonical schema、integrity、identity、cursor、publication boundaryを
encryption前とrestore時に再検査します。Artifactはauthenticated snapshot
boundaryまでのreadingとingest-dedup claimを含められます。Sanitizerは
`target_registry`のdeployment credential tokenを空にし、account、session、device
credential hashはprotected DB stateとして残り得ます。MQTT/TLS private materialは
このDBの外にありartifactへ入れません。暗号化artifactとpassphraseはsecretとして
扱います。

## 6. Handoff、candidate binding、idempotent recovery

`restore`はclosed schema-1 `RecoveryHandoff`だけを受け付けます。Required field
は`recovery_id`、`edge_id`、`edge_node_id`、`old_ledger_epoch`、
`expected_backup_id`、`proposed_new_epoch`、`credential_generation`と
`schema_version=1`です。IDはASCII letters、digits、`.`、`_`、`-`だけで
1..=255 byteです。`old_ledger_epoch`と`proposed_new_epoch`は異ならなければ
なりません。Generationはinteger
`0..=9,223,372,036,854,775,807`です。Handoffはmanifestのbackup ID、Node ID、
old epochへbindします。Restoreはnonnegative Broker credential generationと、
復元DBから読んだdevice-auth generationを記録します。IoTKit Edgeはclosed receiptを
durable recovery caseと照合してからcandidate-bound recovery requestを発行します。
Candidateは両generation、backup、candidate、old/new epoch、Edge cursorを検証して
atomic activation transitionを実行し、不一致はfail closedにします。

Public receiptはclosed schema v2で、statusは`durably_fenced_candidate`、
fieldは`recovery_id`、`candidate_instance_id`、`backup_id`、`edge_id`、
`edge_node_id`、`old_ledger_epoch`、`proposed_new_epoch`、
`credential_generation`、`device_auth_generation`です。Candidate-row provenance（source database
length/digestとencrypted artifact length/digest）はreplay用にprivateにbindし、
receipt、status、audit、errorへ返しません。

Candidate targetはabsolute normalization後にabsent、owner-only、live database
pathと別でなければなりません。Equal name、alias、symlink、hard link、既存の
WAL/SHM sidecarはfail closedです。Offline validationとtyped install operationが
`durably_fenced_candidate` stateへ入った後だけpublishします。Live databaseは
write openせず、このoperationでreplaceしません。

Rename後のexact replay（同じauthenticated artifact bytes、handoff、candidate
binding）はnon-mutating reconciliationで、保存済みreceiptをbyte-for-byte返します。
異なるidentity、handoff、artifact、private provenanceは`candidate_conflict`です。
Renameまたは後続sync/read-backの不確実性はfenced candidateを残して
`candidate_publication_uncertain`を返します。Exact requestのretryだけが許可された
reconciliationです。Retryのためcandidateを黙って削除しません。

## 7. Operator commandとrestore境界

Local-root command shapeは次です。

```text
iotkit-edge-nodectl backup configure --config FILE --db DB \
  --destination DIR --staging-directory /run/iotkit-edge-node-backup \
  --passphrase-file FILE --freshness-seconds 86400 --retention-count 7 \
  --systemd-drop-in FILE [--replace-existing]
iotkit-edge-nodectl backup create --config /etc/iotkit/edge-node-backup.json
iotkit-edge-nodectl backup inspect --input FILE --passphrase-file FILE
iotkit-edge-nodectl backup status --config /etc/iotkit/edge-node-backup.json
```

Create、inspect、statusはbounded nonsecret summaryだけを出します。Passphraseを
argument、shell history、logへ置きません。Deploymentのapproved owner-only手順で
encrypted escrow copyを保管します。Passphraseがないartifactは意図的に復元不能
です。Successful artifactはoff-hostで検証してinspectします。Production restore
drillで実artifactを選ぶのは、§8のとおりIoTKit Edgeがauthenticated inspectionと
Broker fence receiptからmatching handoffを作った後だけです。

```text
iotkit-edge-nodectl backup restore --input ARTIFACT \
  --candidate-db /secure/new/absent-candidate.db \
  --live-db CONFIGURED_LIVE_DB --staging-directory OWNER_ONLY_TMPFS_PARENT \
  --passphrase-file PASSPHRASE_FILE \
  --recovery-handoff VALID_HANDOFF_FILE
```

Candidate pathはcommand前にabsentで、restore後もfencedでなければなりません。
続けて§8のcandidate-bound authorizeとactivateを行います。Restore receiptだけを
通常Node runtimeの起動許可として扱いません。Checked-in handoff/artifact fixtureは
conformance専用で、selected real backupと組み合わせてはいけません。No-backup
hardware replacementはreadingもdedup claimもrestoreしません。

利用可能なauthenticated backupがない場合、またはgateを満たせない復旧をoperatorが断念する
場合、clean replacementは本recovery protocolの外です。旧host、case、nonsecret evidenceを
利用可能な範囲で保持し、旧Broker credentialをfencedのままにし、新しい`edge_node_id`の
fresh DBを初期化して通常commissioningを行います。以前の`recovery_hold`を削除、書換え、
新Nodeの許可として利用しません。別identityの通常commissioningはblockしません。
Sensor identityと設定への影響を定めていたcustody契約は#251 で削除した。本契約自体も#250 で削除する。

Legacy plaintext snapshot fallbackはありません。Former implementation artifact、
renameしたEdge server backup、unauthenticated DB copy、private MQTT/TLS materialを
持つcandidateはこのcontractで受け付けません。

## 8. Production recovery authorityと新epoch activation

Revision 2のproduction journeyではIoTKit Edgeがbackup inspectionとBroker fence
receiptを照合してhandoffを作ります。Broker
receiptはclosed v1で`status=fenced`、`fence_id`、`edge_node_id`、positive
`credential_generation`、`fenced_at`を持ち、password、hash、tokenを含みません。
Fencingはpassword generationを更新してBrokerをrestartするため、旧sessionと旧credentialは
new requestより先に無効です。

IoTKit Edge recovery caseはrecovery/Node/backup/old-new epoch/Broker generation/
snapshot cursor/high-water、authenticated snapshotの正確な`epoch_start` publication
sequence、Edge accepted-throughをdurably bindします。Restore receipt
v2はさらにcandidate instanceとNode device-auth generationをbindします。MQTT request、
result、completion、completion ACKはclosed schema v1、QoS 1、non-retainedです。Edgeは
requestとcompletionをdurable outboxからretryします。Exact replayだけがidempotentで、
field不一致は自動解決せず`recovery_hold`です。

Node applyは一つのImmediate transactionです。Edge accepted-through以下を再送対象から外し、
authenticated old `epoch_start`を除外し、残るold-epoch publicationをnew epochへ連続再採番して
新しい`epoch_start`を一つだけseq 1に置き、ledger
epoch、target cursor 0、resultを同時commitします。失敗時は全rollbackします。Edgeは
matching resultだけでnew epochをactiveにしcursor 0を作成してcompletionをoutboxへ置きます。
Nodeはcompletionをdurably保存した後にmatching completion ACKをpublishします。Edgeはその
ACKを受けるまでcompletion outboxを閉じません。Nodeがcompletionを保存しQoS 1でACKを
publishするまでは通常runtimeを起動しません。
