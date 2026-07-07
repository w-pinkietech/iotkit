# D8: Site-managed multi-gateway topology

Status: 決定 (2026-07-07。複数Pi現場レビュー、subagent細部レビュー、ユーザー裁定を反映)
用語は [../terminology.md](../terminology.md)、責務は [../responsibility-ledger.md](../responsibility-ledger.md) に従う。
入力: [../reviews/2026-07-07-topology-decision-multi-pi.md](../reviews/2026-07-07-topology-decision-multi-pi.md)

## 背景

1サイトに複数のRaspberry Piが必要になる理由は現場ごとに異なる。BravePIメインボード配下の台数上限を超える
容量問題の場合もあれば、BLE到達距離や設置位置の都合でPiを物理分散する距離問題の場合もある。

この事実により、D2 §4の「センサーが遠い場合の衛星アダプタ構成」と、D5が既に認めている
「大域同一性 = `(gateway_identity, system_id)`」のマルチゲートウェイ前提が未突合で共存していることが分かった。

本決定は、複数Pi現場の標準プロダクトトポロジを固定し、D1/D2/D5/D7/R19へ波及する修正を明文化する。

## 決定1: プロダクトトポロジは2つだけにする

### Standalone

Standaloneは、サイト内のGateway Piがちょうど1台の構成である。

- 1台のRaspberry PiがIoTKit Gatewayを動かす。
- YokaKitは同じPiに同梱してよい。
- 上流接続([3]/[4])は任意である。
- Site Console / Site Aggregatorは必須ではない。

2台目のGateway Piを追加する場合は、Standaloneの拡張ではなくSite-managedへの移行として扱う。

### Site-managed

Gateway Piが2台以上必要なサイトはSite-managedとし、site server [3] を必須とする。

- N台のGateway Piはすべて完全なゲートウェイである。
- site serverはサイト単位の運用、監視、アーカイブ、YokaKit、Site Consoleを担う。
- cloud [4] は任意の上位層であり、複数拠点統合、遠隔運用、二次バックアップ、fleet管理を担える。

代表Gateway Pi、親Gateway Pi、または他Piの管理を担う「管理Pi」は標準トポロジとして置かない。
複数Piなのにsite serverを置かない「小規模マルチPi」も標準トポロジにしない。

## 決定2: Site-managedでも各Gateway Piは完全ゲートウェイである

各Gateway Piは以下をローカルに持つ。

- adapter / driver
- collector
- local SQLite
- ingest dedup
- publication log / outbox
- R10 publisher
- R11 local read API
- R12 health / alarm API
- R13 audit / incident bundle
- R14 typed operations
- R15 desired/reported config
- R19 local credentials / certificate state
- R21 update receiver / rollback
- R22 gateway identity / snapshot export / restore
- local UIまたは`gatewayctl`によるbreak-glass操作

各Gateway Piは、自分がUART等で受けたデータを自分のSQLiteへcommitし、そのcommitを最初の耐久点とする。
site serverへ届く前に、Gateway Pi自身がデータのcustodyを引き受ける。

Site-managedは中央ゲートウェイ案ではない。site serverはUARTデータを直接収集せず、Gateway Piのcollectorを代替しない。

## 決定3: site server内のAggregatorとArchiveは別ロールである

Site-managedのsite serverは同一筐体内に2つの論理ロールを持つ。

| ロール | 役割 | custodyへの影響 |
|---|---|---|
| Site Aggregator | 非権威な読み取り、投影、統合表示、運用管理 | custody transferしない |
| Archival Consumer/Store | 各Gateway PiからR10 raw streamを受け、長期保存する | durable archival ackだけがcustody transferする |

Site Aggregatorの受信、表示、投影、キャッシュ更新、YokaKit派生テーブル更新は、Gateway Piのpurgeを許可しない。

Gateway Piのpurgeを許可するのは、Archival Consumer/Storeがraw recordとack cursorを同一の耐久トランザクションで
commitした後に返すarchival ackのみである。

## 決定4: custody状態を明示する

Site-managedの測定データは、次の状態を通る。

| 状態 | 正本 | 備考 |
|---|---|---|
| site archival ack前 | 各Gateway Pi | site側の非耐久受信、Aggregator保持、cloud enqueueは正本移転ではない |
| site archival ack後、Pi purge前 | Site Archival Store | Pi上の残存分は最低保持フロア内の復旧用複製 |
| Pi purge後 | Site Archival Store | raw archival custodyはsite側が負う |
| cloud backup後 | Site Archival Store + cloud複製 | cloudはDR複製。Pi purge許可条件ではない |

site archive損失時、Pi保持内ならbounded backfillで修復できる。Pi purge済みかつbackupなしの範囲は、
Gateway Piの`custody_lost`ではなくsite側の`archive_lost`として扱う。

### archival ack

Archival ackはbatch受信成功ではなく、Gateway Piごとの連続cursor水位で返す。

```text
accepted_through = (gateway_identity, epoch, seq)
```

batch途中で失敗した場合、ackできるのは耐久保存済みの連続prefixまでである。応答喪失時はGateway Piが
同じ範囲を再送し、Site Archival Storeは `(gateway_identity, epoch, seq)` で冪等upsertする。

### purge条件

Site-managedの各Gateway Piは、archive ack済みデータも最低保持フロア以上保持する。

purge可能条件は次の両方を満たすことである。

1. 当該Pi自身のarchival ack水位以下である。
2. 最低保持フロアを超過している。

Aggregator状態、他Piのack、cloud backup成功/失敗はPi purge条件にしない。

## 決定5: R10のmulti-gateway同一性

D7の `(epoch, seq)` はGateway Pi局所の同一性である。Site-managedでは、消費者が保持するグローバルな
record identity、cursor、dedup、ack、backfill、snapshot水位は必ず `gateway_identity` でスコープする。

```text
global_record_identity = (gateway_identity, epoch, seq)
```

`epoch/seq` を単独で消費者DBの主キー、再開位置、dedupキー、ack水位に使ってはならない。

追加規則:

- measurement族のseries参照は、source `gateway_identity` + D5 `series_key` として扱う。
- 消費者が保持するglobal series identityは
  `(gateway_identity, system_id, measurement_key, channel_index, series_variant)` とする。
- `publication_id` はsource gateway + target内のbatch再送冪等キーである。
- batch dedupキーは `(gateway_identity, target_id, publication_id)` とする。
- `target_id` とtarget registryはGateway Pi局所である。
- Site-managedで同一site serverがN台のGateway Piから受ける場合も、R10上はN個の独立target登録として扱う。
- 複数Gateway Pi間にR10上の全順序はない。
- site全体の完全性は単一cursorではなくGateway Piごとの水位ベクトルで表す。
- gap/cursor_expiredは特定 `(gateway_identity, target_id)` の配送制御通知であり、他Gateway Piの配送状態に影響しない。
- annotation族の共有seqはsource gateway内のpublication log共有を意味する。

D7は最低限のmulti-gateway消費者義務を定義する。site server固有のsite snapshot、横断backfill orchestration、
集約R11/R12面、gateway enrollmentはD8または後続のsite-server決定で定義する。

## 決定6: コミッショニングはトポロジで分岐する

### Standalone

- Phase 0: Standalone用イメージを準備する。
- Phase 1-4: 単一Gateway PiのローカルUI/CLIで完了する。
- Phase 5: 上流接続は任意である。YokaKit同梱構成ではローカルR10 targetとして登録してよい。
- Phase 6: サイト内Gateway Piが1台であること、R22退避が構成済みであることを確認する。

### Site-managed

- Phase 0: site serverを先に用意し、`site_id`、期待Gateway一覧、登録トークン、更新ポリシー、
  snapshot退避先を作成する。
- Phase 2: 各Gateway Piは自己構成後にsite serverへ登録する。site serverは証明書fingerprint、
  `gateway_identity`、operator権限、所属siteを記録する。
- Phase 4: デバイス登録はGateway Pi単位で行い、site serverは統合承認画面を提供する。
- Phase 5: 全Gateway Piがsite serverをアーカイブ責任targetとして登録し、Gateway Piごとの
  合成テストパブリケーションを成功させる。
- Phase 6: 全Gateway登録、R10疎通、証明書期限、snapshot鮮度、outbox risk horizon、
  バージョン整合、部分分断なしをサイト単位で検査する。

## 決定7: Site-managedの復旧表をD2へ追加する

| 壊れた箱/状態 | 検知 | 復旧 | データ影響 |
|---|---|---|---|
| Site Aggregator故障/DB損失 | site R12、投影遅延 | Archival Storeまたは各PiのR10から再投影 | ゼロ。custodyなし。Pi収集・purge判断に影響しない |
| Site Archival Store停止/NW断 | 各Piのarchive target配送失敗 | Piは収集継続、未ack範囲を保持、復旧後cursorから再送 | Pi保持上限まではゼロ。長期化時のみR17劣化契約 |
| Site Archival Store DB損失、Pi保持内 | site DB検査/restore時 | cloud backup復元 + 各Piからbounded backfill | 保持内は復旧可。保持外かつbackup外はarchive loss |
| Site Archival Store DB損失、Pi purge後、backupなし | site DB検査/照合 | 復旧不能範囲を`archive_lost`監査として記録 | ack済み/purge済みデータのarchive側損失 |
| Cloud backup損失、site archive健在 | backup監視 | site archiveからbackup再作成 | ゼロ。Pi custody/purgeには影響しない |
| 部分LAN分断 | site serverから見えるGateway集合が期待一覧の一部のみ | 影響Piはローカル保持、未影響Piは通常継続 | 影響Pi保持上限まではゼロ。site viewは部分的 |
| N台中1台のGateway Pi全損 | site R12、当該Pi無応答 | 当該PiのR22 snapshotから復元、旧機の回収/無効化 | 当該Pi配下のみ影響。他Piは無影響 |
| duplicate gateway_identity / stale epoch | enrollment検査、R10認証、epoch不一致 | stale側をfenceし、operator確認 | split-brain防止。ackせず復旧へ誘導 |
| site server交換 | site server無応答、復旧操作 | site backupから復元、各Piの水位ベクトルと再照合 | Pi保持内は各Piから再送可能 |

D2既存の「工場サーバー[3]故障=データ影響ゼロ」は、non-custodial serverに限って正しい。
Site-managedで[3]がArchival Storeを持つ場合は、上表のようにcustody状態ごとに分ける。

## 決定8: break-glassと必須アラーム

site serverが停止しても、各Gateway Piの収集、ローカル耐久化、R9ローカルアクション、R11ローカル参照、
R22手動エクスポートは継続する。

現場作業者はbreak-glass operator権限で各Gateway PiのローカルUIまたは`gatewayctl`に入り、状態確認、
incident bundle生成、サービス再起動、USB snapshot退避を実行できる。site server停止中の変更はローカル監査に記録し、
復旧後にsite serverへ同期する。

Site-managedで必須のアラーム:

| alarm | 意味 |
|---|---|
| `outbox_risk_horizon` | 未ack正本がR17劣化契約の未ack正本破棄に到達する推定残時間 |
| `certificate_expiry` | Gateway証明書、target資格情報、operator tokenの期限またはrotation失敗 |
| `snapshot_staleness` | 最終成功snapshotがRPOを超過、または最終台帳変更後のsnapshot未取得 |
| `mixed_versions` | Gateway間またはsite serverとの契約/API/DB/catalogバージョン不整合 |
| `partial_partition` | site serverから見えるGateway集合が期待Gateway一覧の一部に限られる状態 |

## 決定9: YokaKit統合とR19の優先順位

YokaKitはStandaloneでは同一Pi上のローカルR10 targetとして、Site-managedではsite server上の投影consumerとして動かす。

- YokaKitの`raspberry_pi_id`はsite-local surrogateとして残してよい。
- 安定した外部同一性は`gateway_identity`である。
- `ip_address` / `ipAddress`は互換表示・互換キーに限り使い、認証済み主体、配送元、cursor、権限判断には使わない。
- `gateway_identity -> raspberry_pi_id -> compat_ip_address`の束縛はR19監査対象として管理する。
- YokaKit互換投影はR10 batchを受け、YokaKit側の永続化完了後にackするアプリケーションレベルconsumerである。
- MQTT再publishはR10 archival ackの代替にしない。
- 互換JSONを生成する場合でも、保存時刻は配送時刻ではなくR10の`event_time`を用いる。
- YokaKit完全パリティには`string_observation` familyが必要である。D1改訂前は数値/boolean観測のみの部分移行と呼ぶ。

今回のトポロジ裁定により、R19の直近主戦場はR2入口認証ではなくR10出口認証へ移る。

Site-managedのR19では、gateway enrollment、target registration、credential binding、archive flag変更を先に固定する。

- gateway enrollmentは`gateway_identity`、ledger epoch、証明書fingerprint、site所属を登録する。
- target credentialは `gateway_identity + target_id + target_url + scope` へ束縛する。
- cloud target登録、archive flag変更、資格情報rotation/失効はR14の高権限型付き操作とし、疎通スモークと監査を必須にする。

## 却下した案

| 案 | 却下理由 |
|---|---|
| Model A: 中央ゲートウェイ + rpi4b衛星 | ack耐久点がネットワーク先になり、容量/距離どちらの複数Piにも弱い |
| 小規模マルチPi without server | Pi管理、証明書、target、復旧、長期保存が散らばる |
| 代表Pi / 管理Pi | 中央ゲートウェイと誤解されやすく、故障時にYokaKit/管理面がPiに引きずられる |
| site serverをcollectorにする | Gateway Piのローカルcustodyを壊す |
| cloud backup成功をPi purge条件にする | WAN断でPi purgeが止まり、site archiveの責務境界が曖昧になる |

## 波及修正

本決定により、既存文書へ次の修正が必要である。

1. **D7**: R10のglobal record identityを `(gateway_identity, epoch, seq)` に拡張する。
   `publication_id`、target registry、backfill、snapshot、annotation、gap/cursor_expiredもgateway scopeを明記する。
2. **D2**: コミッショニングをStandalone/Site-managedで分岐させ、Site-managed復旧表を追加する。
3. **D2/D7**: archival ackを `accepted_through=(gateway_identity, epoch, seq)` の連続水位として定義し、
   purge条件をGateway Piごとに明記する。
4. **D1**: `ack=耐久点` を弱めず、custody-criticalなSQLiteトランザクションは `WAL + synchronous=FULL` をMUSTにする。
   `synchronous=NORMAL` は再構成可能なderived/retry metadataに限る。
5. **terminology**: Standalone、Site-managed、Site Aggregator、Archival Store、gateway_identityの用語を追加する。
6. **R19設計**: R10/R19を優先し、gateway enrollment、target credential binding、archive flag操作を先に設計する。

## 保留事項

- `WAL + synchronous=FULL` のRPi上での性能、SD/eMMC/USB SSDごとの書込量、group commit要否は実測する。
- `string_observation` familyはD1/D7の別決定で扱う。
- site-wide snapshot、横断backfill orchestration、複数拠点cloud federationは後続決定で扱う。
