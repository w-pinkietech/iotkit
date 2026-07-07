# D8: Site-managed multi-gateway topology

Status: 決定 (2026-07-07。複数Pi現場レビュー、subagent細部レビュー、ユーザー裁定を反映)
用語は [../terminology.md](../terminology.md)、責務は [../responsibility-ledger.md](../responsibility-ledger.md) に従う。
入力: [../reviews/2026-07-07-topology-decision-multi-pi.md](../reviews/2026-07-07-topology-decision-multi-pi.md)
査読: クロスベンダー2社(codex gpt-5.5 / Claude)に同一プロンプトで査読させ、両者が独立に一致した指摘
(クローンSDのsplit-brain対策・波及修正の実適用・移行手順・break-glass競合)を決定3/4/5/7/8・保留・波及修正へ反映(2026-07-07)。

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

### 同一キー衝突の検知(2026-07-07 査読反映)

`(gateway_identity, epoch, seq)` の冪等upsertは「同一キー＝同一送信元・同一中身」を前提にした最適化である。
予備SDカードの生クローンを本物と同時起動した場合など、**同一キーに異なる中身**が届くと、後勝ちで既存の正本を
無音上書きし、custody_lostを無音で起こす(D2「無音の正本破棄は契約違反」に抵触)。これを防ぐため:

- Archival Storeは各レコードに**中身の指紋(payload hash)**を併せて保存する。
- 同一 `(gateway_identity, epoch, seq)` に**指紋が一致する**再送 → 冪等(正常な再送・backfill。上書きも損失もなし)。
- 同一キーで**指紋が食い違う** → 冪等upsertを行わず**ハードエラーで拒否し、`custody_conflict`(侵害シグナル)として
  監査へ昇格**、operator復旧へ誘導する。この時点で正本は守られている——D2の`custody_lost`(未ack正本の実損。
  欠落annotation必須)とは**別のイベントクラス**であり、欠落annotationは発行しない。
  決定7の「duplicate gateway_identity / stale epoch」行は、
  正規のcert・epochを両機が提示して見分けがつかない同時起動サブケースを、この指紋照合で判別してfenceする。

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

**(D9波及 2026-07-08)** MQTTバインディングでのこの水位の表現: 正式水位は補助topic上の
`accepted_through` 明細であり、成功専用PUBACKはギャップなし状態でのみその累積等価として扱える。
詳細はD9決定2・3。

### purge条件

Site-managedの各Gateway Piは、archive ack済みデータも最低保持フロア以上保持する。

purge可能条件は次のすべてを満たすことである。

1. 当該Pi自身のarchival ack水位以下である。
2. 最低保持フロアを超過している。
3. 対象範囲に `archive_repair_hold`(下記)が掛かっていない。

Aggregator状態、他Piのack、cloud backup成功/失敗はPi purge条件にしない。

### archive復旧中のpurge停止(2026-07-07 査読反映)

Site Archival StoreのDB損失を「Pi保持内のbounded backfill」で修復する間(決定7の該当行)、Piが従来の
ack水位のまま通常どおりpurgeを続けると、backfillで送り直すべき範囲を先に消しうる。したがってsite側の
archive損失/修復を検知したら、対象の `gateway_identity`・範囲へ **`archive_repair_hold`** を掛け、
修復完了までその範囲のpurgeを止める。

**最低保持フロアの起算点**: フロアは各Piの**archival ack観測時刻**を起算とし、event_time起算にも
ローカルcommit時刻起算にもしない。フロアの定義は「ack**後も**置く最低保持期間」(D1)なので、ack観測より
前から数え始めると、site server長期停止後にackが届いた瞬間フロア超過→即purgeとなり、ack後保持がゼロになる。
event_time起算を避けるのは、省電力センサーの遅着バックログをack直後に即purgeしないため。

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
- **消費者は不完全なsite集計を完全値として提示しない**(2026-07-07 査読反映): partial_partition中
  (site serverから一部Gateway Piしか見えない)は、YokaKit/Site Consoleは欠けているPi分を集計から
  除外していることを明示するか、集計自体を暫定として保留する。各Piの到達状況は水位ベクトルで持ち、
  全Pi分が揃うまで確定値として表示しない。`partial_partition` alarm(決定8)だけに頼らない。
- **配送保証はexactly-onceではなくat-least-once + 冪等効果**(2026-07-07 査読反映): 連続prefix水位ackと
  冪等upsertの組み合わせは、単一書き込み元という前提の下で exactly-once 相当になる。前提が崩れる同時起動は
  決定3の指紋衝突検知で受ける。

### gateway_identity の発行・一意性・世代の権威(2026-07-07 査読反映)

`gateway_identity` はSite-managed全体でレコードの出所を決める鍵なので、発行と一意性を明示的に固定する。

- **発行**: 各Gateway Piは初回自己構成(D2 Phase 2)で自分の `gateway_identity` を1回だけ生成する。
  **共有OSイメージには `gateway_identity`・TLS秘密鍵・per-deviceトークンを焼き込まない**——焼き込むと、
  1枚を大量複製したイメージから同一identityの箱が量産され、初期展開の時点で(同時起動を待たず)衝突する。
  イメージに焼いてよいのはサイト共通設定(`site_id`・ネットワーク・更新ポリシー)に限り、台ごとに固有な値
  (identity・鍵・トークン)は必ず初回起動でその場で生成する。
- **一意性検証**: enrollment時にsite serverは同一 `gateway_identity` の重複登録を拒否する。証明書fingerprintと
  `gateway_identity` の対応が既存登録と食い違う登録も拒否し、operator確認へ回す。
- **世代(epoch)の権威**: epochはUUIDで、単調な大小比較ができない(RTCなし前提)。「stale epoch」は大小ではなく
  **site serverが保持するactive epoch台帳との一致**で判定する。site serverはGateway Piごとの現行epochを
  永続保持し、site server交換時はbackupから復元して各Piと再照合する。R22復元で新epochを採番した箱(D2 §3.5)は、
  この台帳更新を経て現行になる。

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
| N台中1台のGateway Pi全損 | site R12、当該Pi無応答 | 当該PiのR22 snapshotから復元、旧機の回収/無効化 | 他Piは無影響。当該Piのsite archiveへ未ackだった正本は失われる(custody_lost)。R22スナップショットはreadings非含有のため新しい箱は喪失範囲を知り得ない——範囲の可視化は欠落annotationではなく、新epoch開始annotation+消費者側カーソル突合による(D7決定8) |
| site serverとGateway Piの同時障害 | site全体無応答 | site serverをbackupから復元→各Piを復旧/再enrollment→水位ベクトル再照合 | 各Piの未ack正本はPi保持内なら残存。両者同時全損の範囲のみ損失(custody_lost) |
| duplicate gateway_identity / stale epoch | enrollment検査、R10認証、epoch台帳不一致、payload指紋衝突(決定3) | stale側/指紋不一致側をfenceし、operator確認 | split-brain防止。見分けのつかない同時起動は指紋照合で判別しfence(無音上書きしない) |
| site server交換 | site server無応答、復旧操作 | site backupから復元、各Piの水位ベクトルと再照合 | Pi保持内は各Piから再送可能 |

D2既存の「サイトサーバー[3]故障=データ影響ゼロ」は、non-custodial serverに限って正しい。
Site-managedで[3]がArchival Storeを持つ場合は、上表のようにcustody状態ごとに分ける。

## 決定8: break-glassと必須アラーム

site serverが停止しても、各Gateway Piの収集、ローカル耐久化、R9ローカルアクション、R11ローカル参照、
R22手動エクスポートは継続する。

現場作業者はbreak-glass operator権限で各Gateway PiのローカルUIまたは`gatewayctl`に入り、状態確認、
incident bundle生成、サービス再起動、USB snapshot退避を実行できる。site server停止中の変更はローカル監査に記録し、
復旧後にsite serverへ同期する。

break-glass中のローカル変更と、site server側の変更が同じ対象(desired設定等)で競合した場合の裁定を定める
(2026-07-07 査読反映)。各変更はrevision(版番号)を持ち、復旧時の同期を**盲目的な後勝ちにしない**。
site側とローカル側の双方に同一対象の変更があれば、新しいrevisionを自動採用せず**競合として保留し、
operator確認へ回す**(緊急対応の変更が無音で上書きされるのを防ぐ)。desired/reportedの調停(R15)に準じる。

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

本決定と**同一コミットで**既存文書へ次の修正を適用する(査読指摘: 決定だけ書いて他文書を直さないとコーパスが
自己矛盾する。本リポジトリの慣行に従い関連文書を同時改稿する)。

1. **D7**: レコード同一性の記述に、Site-managedでは消費者側の global record identity を
   `(gateway_identity, epoch, seq)` でスコープする旨を明記(D8決定5へ委譲)。[適用済]
2. **D2**: §2コミッショニングにStandalone/Site-managedの分岐がD8にある旨、§3復旧表の「サイトサーバー故障=
   データ影響ゼロ」がnon-custodial serverに限る旨、§4衛星アダプタがD8のトポロジ2分と直交(rpi4b代替ではない)
   である旨を注記。Site-managed復旧表はD8決定7。[適用済]
3. **D2/D7**: archival ackを `accepted_through=(gateway_identity, epoch, seq)` の連続水位とし、
   purge条件をGateway Piごとに明記(D8決定4)。[D8内で確定]
4. **D1**: `ack=耐久点` を弱めず、custody-criticalなSQLiteトランザクションは `WAL + synchronous=FULL` をMUSTにする。
   `synchronous=NORMAL` は再構成可能なderived/retry metadataに限る。[適用済]
5. **D5**: series同一性のスコープ宣言に、消費者側の大域同一性が `gateway_identity` でスコープされる旨を注記。[適用済]
6. **terminology**: Standalone、Site-managed、Site Aggregator、Archival Store、gateway_identity、
   active epoch台帳、archive_lost、Site Consoleの用語を追加。[適用済]
7. **responsibility-ledger**: `archive_lost` 監査イベントの定義、R19優先順位(R10出口先行)、R22の
   gateway_identity発行を注記。[適用済]
8. **R19設計(後続spec)**: R10/R19を優先し、gateway enrollment、target credential binding、archive flag操作を
   先に設計する。[後続]

## 保留事項

- **Standalone→Site-managed移行のrunbook**は後続決定で確定する(既存Gateway Piのdata・`gateway_identity`・
  target登録・既存archival consumer指定・既存YokaKit `raspberry_pi_id` の引き継ぎ手順)。当面は、移行時に
  site serverを先に立て、既存Piをそのまま2台目扱いにせず改めてenrollmentし直す前提とする。
- **Gateway Pi decommission**(期待Gateway一覧からの除去・identity無効化・残データの扱い)の手順も後続で確定する。
- `WAL + synchronous=FULL` のRPi上での性能、SD/eMMC/USB SSDごとの書込量、group commit要否は実測する。
- `string_observation` familyはD1/D7の別決定で扱う。
- site-wide snapshot、横断backfill orchestration、複数拠点cloud federationは後続決定で扱う。
