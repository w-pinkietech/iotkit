# D2: データ正本・配置・運用開始/復旧イメージ

Status: 会話合意 (2026-07-02、IoTKit Edge storage profileは2026-07-21確定)
用語は [../terminology.md](../terminology.md)、責務は [../responsibility-ledger.md](../responsibility-ledger.md) に従う。

## 1. データの正本(source of truth)

原則:
- **その情報なしで動けなくなる箱が正本を持つ**
- **測定データの正本は配送とともに移転する(custody transfer)**

| データ | 正本 | 備考 |
|---|---|---|
| 測定データ(時系列) | 移転する: ackの瞬間Edge Node → アーカイブ責任消費者のackで上流へ移転、Edge Node側はパージ可 | 「バッファであって倉庫ではない」の帰結 |
| デバイス台帳・desired設定・測定レジストリ・較正値 | 常にEdge Node | 上流スナップショットは複製。オフラインで閉じるため(柱1)。※「測定レジストリ」の正本は**現場レジストリ**。標準語彙カタログ(リポジトリ資産)との二層関係は [D6](D6-measurement-registry.md) で確定(copy-on-enable) |
| デバイス実状態(reported) | 現実はデバイス、記録はEdge Node | R15 |
| 監査ログ・診断証拠 | 生成はEdge Node、早期に上流へ送出(append-only) | 箱ごと盗難・破損でも証跡が残る |
| センサー意味付け | IoTKit Edge | 保存済みseriesを`production_pulse`等の設定可能な意味へ対応付ける。品番・工程・実績は持たない |
| 業務データ(品番・工程・実績) | YokaKit | IoTKit Edgeの意味付け結果を消費し、業務masterとロジックを所有する(柱2) |
| フリート情報 | [3]/[4]のフリート管理 | Edge Node責務外 |

**アーカイブ責任消費者(archival consumer)**: 出口契約の消費者のうち1つを台帳で指定。
その消費者のackのみが正本移転=パージ許可を意味する。他の消費者のackはパージ判断に関与しない。
上流ゼロの最小構成では正本はEdge Nodeに留まり、retention期限が正本の寿命(明文で受け入れる)。

**劣化契約との優先順位(2026-07-03確定、外部レビュー第2回反映。同日D7決定7により4クラスへ改訂)**:
R17劣化契約のパージ順序は
**①アーカイブ責任消費者ack済み(最低保持フロア超過分)→ ②保管対象外シリーズ(D7の保管対象ポリシー。
窓超過分は常時削除、圧力時は窓内も監査イベント付きで削除可)→ ③解決もretireもされない検疫滞留行(D5)→
④未ackの正本**の順。検疫かつ保管対象外の行は②に帰属(custody約束がない方が支配)。
④に到達したときのみ「データ損失」であり、④の実行には
**custody_lost監査イベント**(対象範囲・件数・当時の水位を記録)と、出口契約への**欠落annotation**
(消費者が欠落区間を構造化データで知れる)を**必須**とする。無音の正本破棄は契約違反。

## 2. 初期化・運用開始(コミッショニング)

以下は単一Edge Node(D8のStandalone)を前提とした標準手順である。**IoTKit Edgeへ接続する構成(D8のEdge-connected)では
コミッショニングがトポロジで分岐する(D8)。** また共有OSイメージには `edge_node_id`・TLS秘密鍵・トークンを
焼き込まず、台ごとに固有な値はPhase 2の初回自己構成で生成する(D8。大量複製イメージからの同一identity量産を防ぐ)。

- Phase 0 準備: SDにOSイメージ(A/B構成済み)を焼く。オフィスで可
- Phase 1 物理設置: RPi+HAT+電源+センサー配置
- Phase 2 初回自己構成(**全自動**, R22): 機体ID生成・自己署名証明書生成・DB初期化。管理者所有権が
  未確立の間はネットワーク制御API/UIをbindせず、mDNSにも管理面を公開しない(2026-07-12 Plan 6裁定)。
- Phase 3 初期設定: 箱上の `iotkit-edge-nodectl`(物理/SSH root)で管理者パスフレーズを非echo入力して所有権を
  1回確立する。その後にスマホ/PCのUIを開き、時刻確認・ネットワーク設定以降を行う。ネットワーク越しに
  初期所有権をclaimする経路は設けない。
- Phase 3.5 レガシー移行(該当現場のみ、**Phase 4より前**に実施。D5波及 2026-07-02): 移行エントリの播種(D5経路D)。
  先に播種することで、Phase 4の自動検出はhardware_idマッチで移行済みエントリに解決される
- Phase 4 デバイス登録(D5決定4の3経路で書き分け):
  - A: 自動検出→**目撃ステージング**→承認(採番)→検疫→active
  - B: 位置識別型は**位置の定義=登録**
  - C: 自作機は `device add` でトークン発行と同時に採番(ペアリングウィンドウ+登録コード。
    儀式の精密化=D11決定6: 登録コードは単回使用・短TTL、窓は自動クローズ、流量クラスは既定値可)
- Phase 5 上流接続(任意): YokaKit/アーカイバ設定→疎通スモークテスト
  (**検疫の影響を受けない合成テストパブリケーション**で行う)。AIハーネスへのoperatorトークン発行
- Phase 6 開始チェック: Edge Node自身が自動検査し「導入完了レポート」生成。追加項目(レビュー反映 2026-07-02):
  - **流量クラス申告合計の検算**(D11決定4 2026-07-08): 全デバイスの申告流量の合計がこの箱の実測体力に
    収まるかを検算。超過は人間の明示承認(`capacity_debt` 記録)がない限り不合格。検算はPhase 6限りでなく
    `device add`・クラス変更のたびにも実行される
  - **初期検疫の解決**: 検疫滞留デバイスを列挙し、一括解除操作または自動失効の残時間を提示
  - **スナップショット退避の構成+初回退避成功の確認**(オフライン構成は手動USBエクスポート運用。
    頻度目安とリマインド動線を定める)

基準: 初期所有権確立のローカル1操作を除き、AIハーネスなしでもUIで完了できる(第一波)。AIがあれば
所有権確立後のPhase 3-5を対話案内。量産向けper-card provisionerはPlan 6に仮定せず、外部配布前の
独立deliverableとする。

## 3. 物理デバイス異常時の復旧

| 壊れた箱 | 検知 | 復旧 | データ影響 |
|---|---|---|---|
| センサー[1] | 死活(R7)→通知 | 電池交換/交換機→台帳でhardware_id差し替え、**seriesは継続** | 欠測マーキング+当該series較正の**要再確認**(D5ガードレール) |
| 親デバイス(BravePI Mainboard等)[1] | 配下子デバイスの一斉死活 | 一括replace+メンテナンスウィンドウ(D5ガードレール5) | 配下全子の区間欠測 |
| IoTKit Edge Node[2]全損 | 上流/AI無応答+LED | 予備RPi+最新スナップショット復元(R22)→デバイス自動再認識 | 未配送outbox分+最終スナップショット以降の台帳変異を損失(明文で受け入れ) |
| IoTKit Edge Node[2]部分故障 | 監督(R20) | 自動再起動→degraded+AI診断→操作カタログ | 当該アダプタの区間欠測 |
| IoTKit Edge[3] | Edge Node無影響 | IoTKit Edge再構築+接続再設定 | 未ack範囲はEdge Nodeから再送。IoTKit Edgeが既にcustodyを取った範囲の損失はIoTKit Edge側`archive_lost`(D8) |
| 上流断・NW断 | 接続状態機械 | 何もしない→復旧後カーソルから自動再送 | ゼロ(長期断のみ劣化契約) |

設計思想: **どの箱が死んでも復旧は「交換して、スナップショットか自動再認識で戻す」に統一**。
デバイス/現場設定の手作業再入力は復旧経路に含めない。ただし権限の安全な再確立は例外とし、
2026-07-12 Plan 6裁定によりlocal admin recoveryとoperator token再発行を必須とする。

### IoTKit Edge backup・復元境界 (2026-07-21追記)

IoTKit EdgeはEdge Nodeから`accepted-through`を返した時点以降のraw archive正本を持つ。したがって、古いIoTKit Edge backupを
復元すると、backup後にIoTKit Edgeが受理してEdge Node側で既にpurge可能になった区間は自動再送できない場合がある。
DB fileだけを戻して通常起動し、欠番を無視したりcursorだけを現在値へ進めたりしてはならない。

v1は次を採用する。

1. IoTKit Edge backupはSQLiteの整合snapshot、format version、IoTKit Edge ID、schema version、作成時刻、DB hash、
   Edge Node別accepted cursorを一つの暗号化containerへ入れる。IoTKit Edge account password hash、session hash、監査、
   device情報を含むため、平文backup成功経路を持たない。復旧passphraseは所有者限定fileからだけ読む。
2. backup作成後にsnapshotの`quick_check`とmanifest/hashを検証し、同一filesystem上の一時fileから原子的に
   完成名へ切り替える。既存backupを黙って上書きしない。
3. 復元は既存DBを上書きせず、新しいDB pathへ展開・検証する。全IoTKit Edge sessionを失効し、復元metadataを
   DB transactionへ記録してから完成扱いにする。Broker credential、certificate、private keyはcontainerへ
   入れず、deployment設定から再接続する。
4. 復元DBのcursorより先から同じEdge Node/epochのbatchが届いた場合、通常のgapとは分けて
   `archive_recovery_required`を耐久記録し、ackを返さず当該Edge Nodeを`recovery_hold`にする。Consoleは失われる
   可能性のある`backup cursor + 1 .. incoming cursor start - 1`を表示する。
5. v1はEdge Nodeのaccepted済み行を巻き戻して再送するprotocolを持たない。IoTKit Edge hostのlocal CLIで範囲を確認し、
   人間が明示承認した場合だけ`archive_lost`監査を同じtransactionで記録してIoTKit Edge cursorをgap直前まで進め、
   Edge Nodeを再開する。これにより損失は起こり得るが、無音の損失と永久retryを防ぐ。将来のretained replayは
   terminal/gap repair protocolとして別versionで追加する。
6. raw retentionは、対象rawを含む検証済みbackupが存在し、意味付けprojectionが完了し、未配送outboxを
   削除しない場合だけ実行できる。容量watermarkを理由にこの順序を飛ばさない。初版は自動purgeを既定offとし、
   保存状況と削除可能範囲を先に可視化する。

### IoTKit Edge storage profile (2026-07-21確定)

IoTKit Edgeは導入規模に応じて次の2つのstorage profileを持てる設計とする。profileは信頼性や機能の等級ではなく、
配置、運用可能な容量、backup方式の違いである。どちらも同じMQTT契約、Edge Node activation、raw/cursor custody、
意味付け、account、監査、Output Adapterの動作を提供し、検証済みcapacity envelope内では正式運用に使える。

| profile | 単一の正本DB | 主な配置 | 運用上の性質 |
|---|---|---|---|
| `embedded` | SQLite | Raspberry Piへの全部入り、小型PC、低〜中流量のLinux Edge host | DB daemonとDB credentialを増やさず導入できる。local SSD等、SQLite WALに適したhost-local storageを必須とする |
| `postgres` | PostgreSQL | 高流量、長期保持、複数利用者、または別DB hostを必要とするLinux Edge host | server DBの監視、credential、backup、version更新を伴う代わりに、read/write concurrency、partitioning、PITR/replicationへの発展余地を持つ |

次を不変条件とする。

1. 一つのIoTKit Edgeは一時点で一つのprofileと一つの正本DBだけを使う。SQLiteとPostgreSQLへのdual write、
   自動fallback、外部時系列DBを含む複数正本を作らない。
2. profileは導入時に選択してdurable metadataへ固定する。Consoleから稼働中に切り替えない。接続不能時に別profileを
   空DBとして起動せず、fail closedする。
3. storage実装は共通の適合testを通す。最低でもrawとaccepted cursorの同一transaction、exact replay、content conflict、
   commit失敗時ack禁止、activation、revision precondition、監査、outbox、backup/restore fenceを同じ観測可能な契約にする。
4. `embedded`から`postgres`への移行はoffline operationだけとする。新規入力を止め、整合backupを作り、全table、hash、
   Edge Node別cursor、pending outbox、account/auditをimportし、件数とidentityを検証してから切り替える。元DBは検証済みの
   rollback資産として保持し、移行先と同時稼働させない。
5. TimescaleDB等のPostgreSQL拡張は第3のprofileや別正本にしない。plain PostgreSQLの実測上限不足、または圧縮、
   time partition、continuous aggregate等の具体的要件が確認された場合だけ、`postgres` profile内部のschema選択として
   別途決定する。IoTKitの時刻非依存record identityを弱めてhypertableへ合わせてはならない。
6. profile選択はsensor台数だけで決めない。Edge Node数、合計records/秒、burst、payload size、rule/output fan-out、
   保持期間、同時Console/CSV利用、backup時間、RPO/RTOを含む実測capacity envelopeで決める。

初期実装順は`embedded`を基準実装として契約とcapacityを確立し、同じ適合testを満たす`postgres`を追加する。
現在のv1 release candidateは`embedded`だけを実装している。`postgres`未実装を理由に検証済み範囲内の小規模運用を
禁止しない一方、大規模運用を無根拠に保証しない。

## 3.5 監査追記: スナップショットの機密性とエポックフェンス(2026-07-02)

「予備RPi+スナップショット復元→デバイス自動再認識」を成立させるには、スナップショットに
**TLS秘密鍵とデバイストークン(ハッシュ)を含める必要がある**(含めないと、ピン留め済みの全デバイスが
新証明書を拒否し「自動再認識」が嘘になる)。したがって:

- スナップショットは**高機密資産**として扱う。暗号化必須(リカバリパスフレーズ方式)、退避先のアクセス制御。
- 退避経路は出口契約(R10)とは別の専用チャネル(R22の一部として設計スペックで確定)。
- 復旧表の[3]故障「データ影響ゼロ」は、通知がEdge Node配置であるという確定判断
  (台帳プロダクト判断)に依存している。
- **エポックフェンス(D5波及 2026-07-02)**: 全損復旧の前提条件として**エポックフェンス(台帳世代番号)**を
  定義する(下記最小契約で確定)。復旧runbookには**旧機の物理回収/無効化ステップを必須**で含め、
  旧エポックの箱からのackと出口配送を拒否する(旧機復活スプリットブレイン対策。D5より問題提起)。

### R22最小契約(2026-07-02確定。相互依存の解消=外部レビュー指摘反映)

Wave 0のR22(手動エクスポート)が曖昧なまま実装されると復旧・重複配送・秘密管理を後で壊すため、
最小限を先に固定する:

1. **台帳エポック**: ledger_metaに保持するUUID。**スナップショット復元時に必ず新エポックを採番**する
   (復元=新世代)。旧エポックを名乗る箱からの出口配送・制御面操作は拒否(フェンスの実体)。
   エポック不一致を見た消費者はreplay/backfill再交渉(D5決定3)。
2. **スナップショット内容**(manifest+`format_version` 付き、初版から):
   ①論理`edge_node_id` ②devices/series台帳 ③現場レジストリ ④較正値 ⑤desired設定 ⑥`secrets` セクション
   (TLS秘密鍵・per-deviceトークンハッシュ・operatorトークンの**失効済み監査metadata**・
   **`self_managed_static` のトンネル秘密鍵**。**Wave 1+**)。`managed_overlay` providerのnode秘密/stateと
   MQTT broker credentialはsnapshotへ含めず、
   復元後にIoTKit Edge operatorが再設定する(D10)。
   readings本体は対象外(custody transfer/outboxの領分)。
   **2026-07-12 Plan 6改訂:** per-deviceトークンは可用性優先で有効なまま引き継ぐ。これはStandaloneで
   snapshot取得後の失効が巻き戻り得ることを明示受容し、復元時に騒がしく報告する契約である。一方、
   admin credential・operator token・sessionは有効復元しない。復元先の当該権限を消去し、新auth epochの採番と
   復元DB状態を同一Txでcommitする。箱上のlocal admin recoveryとoperator token再発行を要求する
   (従来の「復旧直後に既存operator tokenで入れる」保証を破棄)。
3. **暗号化の条件**: `secrets` セクションが非空のスナップショットは暗号化必須(リカバリパスフレーズ方式)。
   Wave 0の手動エクスポートはTLS/トークン未実装のためsecrets空=平文JSONで可。ただし
   `format_version` と `secrets` 予約セクションを**初版から**持つ(後からのフォーマット破壊を防ぐ)。
   Plan 6でdevice tokenが存在した後、Plan 6.5の暗号化container実装までlegacy平文exporterは
   R22 replacement backupの作成を拒否する。tokenを省いた成功表示も、hashの平文出力も不可。
4. 復元手順: スナップショット流し込み→新エポック採番→デバイス自動再認識(hardware_idマッチ)→
   出口消費者は新エポックを観測して再交渉。旧機の物理回収/無効化はrunbook必須ステップ。
5. **DB/TLS世代フェンス(2026-07-12 Plan 6改訂):** 復元はDBと置換対象外の場所にdurableな
   `restore-in-progress` をfsyncし、論理edge/auth/ledger世代と選択TLS fingerprint/世代を束縛する。
   全てが一致するまで起動はnetwork-unbound。中断時は復元を再開/修復するかunboundを維持する。
   Plan 6はこのfail-closed契約とDB内権限の原子的閉鎖を固定/実装し、Plan 6.5がcross-filesystem
   staging・fsync・rename・フェンス回復mechanicsと暗号化containerを実装する。

## 4. アダプタ・コレクタの配置

- **コレクタ: 常にEdge Nodeと不可分**。ackの瞬間にデータの正本が誕生する場所であり、切り離すと正本の定義が壊れる。
- **アダプタ: デフォルト同梱、ただしアーキテクチャ要件ではない**。
  - 物理ポートに縛られるアダプタ(UART HAT/I2C)は通常Edge Node同梱(運用する箱は少ないほど良い)
  - センサーが遠い場合は**衛星アダプタ構成**: RPi Zero等でアダプタランタイムのみ動かし、
    HTTPバインディングでコレクタへ送る。自作デバイスと完全に同じ経路・同じ契約(D1のバインディング複数の帰結)。
    **経路はローカルLAN内のみ**(D11決定7 2026-07-08: 入口リスナーはインターネット非公開のため、
    インターネット越えの衛星アダプタは構成として認めない。遠隔地は別のIoTKit Edge Node+出口契約で運ぶ)
  - ソフトウェアは同一、デプロイ配置だけの違い
  - **注(D8 2026-07-07)**: この衛星アダプタ(コレクタを持たない薄いランタイム)は、D8のトポロジ2分
    (Standalone/Edge-connected)とは**直交する概念**である。D8が却下したのは「rpi4b級の完全なEdge Nodeを
    中央1台の衛星にする Model A」であって、RPi Zero級の非コレクタ衛星アダプタではない。複数Piが
    それぞれ完全なEdge Nodeになる構成(=Edge-connected)の各Piが、さらにその配下にこの衛星アダプタを
    持つことはあり得る。
