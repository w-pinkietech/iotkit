# D3: プロセス決定とWave分割

Status: 確定 (2026-07-18、v1ゴールと完了条件を追記)

## 現在のプロダクト中心価値

> IoTKitは、さまざまなセンサーを小さなadapterで簡単につなぎ、IoTKit Edgeで現場データを
> 黙って失うことなく収集・正規化・保全・再送し、IoTKit Siteへ届けるIoTデータ収集基盤である。

この価値は次の2点の組み合わせにある。

1. **adapterの作りやすさ**: adapter作者はセンサー固有の通信、設定、値の読み取り、measurementへの
   写像だけを担当する。SQLite、MQTT、再送、保管責任、retention、認証をadapterへ持ち込まない。
2. **保管責任の確実な引き渡し**: Edgeは観測を耐久保存し、Siteがraw recordを耐久保存したと確認できる
   まで保持する。MQTT PUBACKだけではEdgeの保管責任を解除しない。

IoTKitは汎用IoTプラットフォーム、ダッシュボード、生産管理、独自MQTT brokerではない。R1〜R23と
Wave 1/2は将来の責務地図であり、現在すべてを実装するバックログではない。

## v1ゴール

> PinkieTechの導入支援を受けた1つの工場で、IT専任ではない現場担当者が、センサーの状態と
> 現在値を確認し、用途に応じた意味付けを設定し、外部applicationへMQTTで継続的にデータを
> 渡せる。通信断、再起動、一時的な障害が起きても、耐久保存したデータを黙って失わず、
> Site Consoleと手順書から状態確認と復旧ができる。

対象topologyは、1つのIoTKit Siteに1台以上のIoTKit Edgeとする。1工場内の複数EdgeはSiteが扱う。
複数工場を横断した統合管理はYokaKit等の上位applicationの責務であり、IoTKit v1へ含めない。

完了判定は、機能の存在だけでなく、各条件に対応する実行可能test、画面、CLI出力または手順と、
最後の新規導入シナリオを証拠にする。focused testを開発中に使い、全体testと実機を含む最終ゲートは
変更をまとめた後に一度実行する。

### v1完了条件

| 領域 | 完了条件 | 必要な証拠 |
|---|---|---|
| 新規導入 | 既存DB、設定、credentialを流用せず、新しいLinux Site hostとRaspberry PiへEdge、Broker、Siteを導入できる。最初の`system_admin`をSite hostのlocal CLIで作成し、descriptorで発見したEdgeをSite Consoleでactivationした後、commissioning smokeをSite受理まで通せる。完全無人導入と、IoTKitによるLAN、DNS、router、firewall構築は要求しない | 新規一時環境の導入test、公開CLIの出力、1ページから辿れる導入・診断手順 |
| 実センサーとadapter境界 | BravePI MainboardのUARTから温度と接点入力を取得し、Edgeで耐久保存してSiteへ渡せる。BravePI固有処理はadapter内へ閉じ、疑似adapterまたは別adapterが同じingest契約を使える | Raspberry PiとBravePIによる温度・接点の実信号証拠、host上のadapter境界test |
| 複数Edge | 1つのSiteへ複数Edgeを同時接続でき、identity、credential、topic、raw record、cursor、表示が混ざらない | BravePI実機Edge 1台の試験に加え、host上で疑似Edge 2台を同時接続する統合test |
| Site Console | 工場LANのWindows browserからCaddy HTTPS経由で利用できる。全画面でloginを要求し、個人accountと`viewer`、`admin`、`system_admin`を分離する。Edge、device、signalを別階層で表示し、descriptorで発見したEdgeの初回Site activation、sensor type、現在値、最終受信時刻、停止・古い・未設定状態を日本語で確認できる。表示名と設置場所を設定でき、変更者と変更内容を監査できる | browser E2E、activation権限・再送・未登録recordsのnegative test、監査と秘密非表示test、現場担当者役による操作確認 |
| Siteでの意味付け | IoTKitの汎用語彙として、数値の現在値、booleanの現在状態、接点・光等から作る累積値、補正、しきい値による状態・alarmを扱える。設定はfuture-onlyで、過去値を暗黙に再計算しない。保存前に実信号previewで挙動を確認できる | evaluatorの境界test、Console/API E2E、mapping revisionとfuture-only test |
| 外部application出力 | Siteの意味付けと、出力先固有のpayload変換を分離し、Output Adapterを追加できる。YokaKit Adapterは合意済みの`source-id`、`signal-id`、observation contractを使い、IoTKitの累積値を`kind=production`へ、boolean状態を`onoff`または`gantt_chart`へ、alarm判定を`alarm`へ変換し、source statusも送れる。barcodeはIoTKit v1で直接取り込まない | YokaKit契約fixture、YokaKitとのconsumer contract test、YokaKit非依存のOutput Adapter境界test |
| 配送と保管責任 | Site activation後にpublication admissionされたrecordはSiteの`accepted-through`までEdgeが保持する。登録前ローカル値はpublication identityを持たずSiteへ送らない。Siteが受理したraw recordは再起動後も残る。外部出力eventはMQTT PUBACKまでSite outboxへ残り、at-least-once再送時に同じeventを識別できる | Edge/Site/Broker再起動、activation境界、未登録records拒否、経路断、重複、DB書込失敗、outbox再送・収束test |
| MQTTと証明書 | MQTTはTLS、匿名禁止、主体ごとのcredential、exact topic ACLを使う。Mosquitto server certificate lifecycleはIoTKitの意味付けやYokaKitに依存しないBroker-host運用componentが担う。完成済みbundleのinstall経路と`lego` ACME経路を持ち、検証、原子的切替、reload、新規TLS/MQTT probe、失敗時rollback、期限statusまで自動化する | 実Brokerの認証・ACL・TLS negative matrix、Pebbleを使う自動更新test、更新失敗とrollback test、期限status |
| Siteの認証と復旧 | password、credential、秘密鍵をlog、監査、画面、Gitへ出さない。初期所有権と緊急password復旧はSite hostのlocal CLIだけで行い、sessionを失効できる。Site ConsoleはHTTPS失敗時にHTTPへfallbackしない | account/session統合test、secret scan、local CLI復旧test、Caddy障害test |

「データを黙って失わない」の検証境界は次とする。

1. Site activation後にpublication admissionされたrecordは、Siteが同じrecordを耐久保存して有効な
   `accepted-through`を返すまでEdgeが保持する。activation前のローカル確認値はpublication identityを
   持たず、Site custodyや後日replayの対象にしない。
2. Edgeで容量不足等により耐久保存できなかった入力は成功応答せず、operatorが診断できる状態を残す。
3. Siteが受理したraw recordとcursorは同じtransactionで確定し、commit失敗時は
   `accepted-through`を返さない。
4. Siteのapplication eventは、出力先BrokerのPUBACKまでoutboxへ残す。application固有の業務処理成功を
   IoTKitの配送成功とは偽らない。

### v1最終リリースゲート

新規環境から、次の一本を通す。

```text
BravePI温度・接点
  -> IoTKit Edge
  -> MQTT Broker
  -> Site ConsoleでEdgeを発見・activation
  -> IoTKit Site raw保存
  -> Site Consoleで現在値と状態を確認
  -> 累積値・boolean状態・alarmを設定
  -> YokaKit Adapterから合意済みMQTT契約で出力
  -> Edge/Site/Broker再起動と通信断
  -> 復旧後にraw cursorと未配送outboxが収束
```

この実機Edge 1台のシナリオに、host上の疑似Edge 2台同時接続、Site DB書込失敗、MQTT認証失敗、
誤CA・誤hostname・期限切れcertificate、sensor信号停止、password紛失時のlocal CLI復旧を加える。
現場担当者役がWindows browserからloginし、現在値確認、意味付け、出力確認、障害状態の把握を
手順書どおり完了できた時点でv1完成とする。

### v1に含めないもの

- 複数工場の統合管理とYokaKitの業務機能
- camera映像の外部出力とbarcodeのIoTKit直接取り込み
- OTA、fleet更新、MQTT Broker HA、完全無人導入
- あらゆるsensorのadapterとadapter code generator
- mTLS必須化、Keycloak/OIDC連携、短命credentialの完全自動rotation
- Site ConsoleからのBroker endpoint、certificate、credential、ACLの作成・編集・切替
- IoTKitによるLAN、DNS、router、firewall、VPN構築

## 最初の実装ゲート: 実機縦切り (完了)

Wave 1の残項目を横に広げる前に、次の1本を完成させた。

```text
Temperature sensor
  -> BravePI Transmitter
  -> BLE Long Range
  -> BravePI Mainboard
  -> UART
  -> IoTKit Edge on Raspberry Pi
  -> MQTT QoS 1
  -> MQTT Broker
  -> IoTKit Site
  -> raw SQLite
  -> direct CLI query
```

### 作るもの

- BravePI MainboardのUARTを受動的に読む既存adapterと、温度frameのdecode。手元の機体は
  sensor type 261(MCP9600熱電対温度)と仮定して開始し、最初の実frameで型を確認する
- `device_number`を安定したhardware identityとして扱い、RSSIとbatteryを観測へ引き継ぐ経路
- Edgeでのcanonical record化、readingとoutboxのatomic保存
- EdgeのMQTT publish、`accepted-through`検証、cursor更新、保管責任に基づくretention
- Mosquitto等のMQTT Brokerと、Edge Nodeごとのstatic credential/topic ACL
- Siteでのraw recordと連続cursorのatomic保存、commit後の`accepted-through` publish
- 重複再送の冪等処理と、同一identity・異内容のconflict検出
- Site raw dataの直接CLI query
- Dockerによる開発・障害試験環境と、Raspberry Pi上のsystemd実行

### 完了条件

- ペアリング済みBravePI Transmitter 1台の温度観測が、UART経由でEdgeとSiteの双方に記録される。
- 実frameからsensor typeとpayloadを確認し、仮定と異なる場合も正しいdriverへ明示的に対応付ける。
- `device_number`、RSSI、battery、温度値が期待どおりdecode・保存される。
- EdgeまたはBravePI Mainboardの再起動後、UART受信が自動的に復帰する。
- 保持容量内では、Site、Broker、ネットワークの停止中もEdgeが収集を継続し、復旧後に欠けなく再送する。
- 同じbatchの再送でSiteのraw rowが重複せず、異内容ならconflictとしてcursorを進めない。
- SiteのSQLite commit失敗時は`accepted-through`を返さない。
- MQTT PUBACKだけではEdge cursorやpurge eligibilityが進まず、検証済み`accepted-through`だけで進む。
- Edge、Broker、Siteを個別に再起動しても同じ状態へ収束する。
- Docker試験に加え、Raspberry Piと実センサーで一連の流れを再現できる。

### 実機検証状況 (2026-07-13)

Raspberry Pi (Debian 13 / arm64) とペアリング済みBravePI実機で、現在のEdgeから
Siteまでの縦経路を確認した。

- `/dev/serial0` (`ttyAMA0`, 38400 8N1) から既存POCで、温度センサー
  `246880020140018b` と接点入力センサー `246880020140018c` のframeをdecodeした。
  温度、接点の0/1、RSSI、batteryが取得でき、decode errorは観測されなかった。
- 温度センサーはEdge上で `hardware_id = ble:246880020140018b`、
  `measurement_key = temperature_c` へ正規化された。未登録状態の45秒間に4観測が
  sightingと`staged_readings`へ保存され、温度25.5625〜26.1875°C、RSSI -68〜-66 dBm、
  battery 100%を確認した。
- CLIでsightingを承認してdeviceをactive化した後、次の45秒間に4観測が非検疫の
  `readings`へ保存され、対応する4行が`publication_log`へ同じ順序で作られた。
  温度は26.0〜26.3125°Cだった。
- 停止後のSQLiteに`PRAGMA quick_check`を実行し`ok`を確認した。試験終了後はEdgeが
  停止し、UARTが解放されることも確認した。
- 同じPi上の独立したDocker環境で、認証・topic ACL付きMosquittoとIoTKit Siteを起動した。
  Edgeに保持されていた`pub_seq` 1〜8をMQTT QoS 1で送り、Site raw SQLiteの8行と、
  Site commit後の`accepted-through = 8`によるEdge cursor更新を確認した。
- UART収集とMQTT出口を同時に35秒動かし、新しい温度3観測が`pub_seq` 9〜11としてSiteへ
  保存され、Edge cursorも11へ進んだ。Site CLIからraw recordを直接照会できた。
- broker再起動時、Siteが再接続後にrecords topicを再購読しない不具合を実機で発見した。
  購読をPahoの接続callbackへ移し、broker再起動をend-to-end試験へ追加した。修正版Siteは
  containerを再起動せず再購読し、その後の実機配送と`accepted-through`を完了した。

2026-07-14にはEdge/Site命名変更後のcommit `c0826a0`を使い、実験用labを旧DB・旧設定・旧MQTT
credentialごと削除して新構成から再作成した。新しい`edge_node_id`と専用credential/topic ACLを生成し、
温度センサー`ble:246880020140018b`を新DBで承認・active化した。BravePI MainboardのUARTから受信した
新しい温度観測11件が`pub_seq` 1〜11としてIoTKit Siteの11行へ同じ`edge_node_id`・`ledger_epoch`で
保存され、Edgeの`accepted-through` cursorも11へ収束した。停止後のEdge DBとSite DBはともに
`PRAGMA quick_check = ok`であり、Edge停止後にUARTが解放されたことも確認した。破棄したpre-releaseの
identity key、topic namespace、credentialは再利用していない。

同日の最終レビュー修正後commit `b6b9402`でも、上記の新形式DBを再作成せずに最終Edgeを起動した。
read-only cutover preflightが既存の`edge_node_id`を受理し、BravePI MainboardのUARTから受信した
温度3観測が`pub_seq` 12〜14としてSiteへ保存された。Edgeの`accepted-through` cursorは11から14へ進み、
Site queryで同じ`edge_node_id`・`ledger_epoch`と`temperature_c`の3 recordを確認した。停止後のEdge DBと
Site DBはともに`PRAGMA quick_check = ok`であり、Edge processの停止とUART解放も確認した。

### Host耐障害検証状況 (2026-07-14)

commit `0696d22`を使い、通常のEdge、Mosquitto、Siteだけでcustody失敗系とprocess境界の耐障害性を
検証した。テスト専用の製品failpointは追加していない。

- Siteの実SQLite transactionをtriggerで失敗させた場合、raw record、cursor、ackはいずれも0件のまま
  だった。同じbatchの再送は1行のまま冪等で、同じidentityを異なる内容で再送した場合はconflictとなり、
  元recordのbytesとcursorを保持して2つ目のackを返さなかった。
- Brokerを停止した状態でEdge outboxへ300件を作成し、Edgeを再起動してもcursorが0、outboxが300件の
  まま保たれることを確認した。BrokerとSiteの復旧後、256件上限により2 batch以上でcursorとSiteが
  300へ収束した。
- この試験で、IoTKitの1 MiB batch上限に対してMQTT clientの既定送信上限が10 KiBのままという不整合を
  検出した。client上限を最大payloadとtopic/header overheadへ揃え、約42 KiBの先頭batchを正常送信した。
- Brokerが利用可能なままSiteを停止し、`pub_seq` 301を追加してEdgeを再起動してもcursorは300から
  進まなかった。Site復旧後は301へ収束した。その後、Edge再起動をまたぐ302、Broker再起動をまたぐ303、
  Site再起動と通常retryをまたぐ304を順に確認した。
- 最終状態はEdge cursor 304、Site raw record 304件、最小1、最大304、distinct 304であり、欠番と重複は
  なかった。停止後のEdge DBとSite DBはいずれも`PRAGMA quick_check = ok`だった。

### 実機耐障害検証状況 (2026-07-14)

commit `3efd599`のsourceをRaspberry Pi (arm64)へ同期してdebug buildし、温度センサー
`ble:246880020140018b`とBravePI Mainboardを使って、hostでは代替できないUART復旧だけを確認した。

- 開始時はEdge停止、Broker/Site稼働、Edge readings 18件、current-epoch `pub_seq`最大18、Edge cursorと
  Site raw recordは17だった。Edge/Site DBはいずれも`PRAGMA quick_check = ok`だった。
- Broker/Siteを停止して最新Edgeを起動すると、cursor 17のままreadingsは18から22へ増えた。さらに
  EdgeをSIGINTで停止して再起動し、修復操作なしでreadingsが26から32へ増えたため、downstream停止中の
  SQLite収集継続とEdge再起動後のUART再取得を確認した。
- Broker/Site復旧後、停止中のrecordを再送してEdge cursorとSiteが17から39以上へ追いついた。その間も
  UART収集は継続し、新しいrecordだけが通常の短い送信待ちになった。
- ユーザーがBravePI Mainboardを一度power cycleした。再投入直後の基準readings 117から35秒後に123へ
  増え、再ペアリング、設定修復、DB修復、device再承認なしで`temperature_c`受信が自動復帰した。
- 最終的にEdge readings 127件、current-epoch `pub_seq`最大127、validated `accepted-through` cursor 127、
  Site raw record 127件（最小1、最大127、distinct 127）へ収束した。Edge/Site DBはいずれも
  `PRAGMA quick_check = ok`であり、EdgeをSIGINTで停止した後にUARTが解放された。Broker/Site、lab、
  credential、source copyは削除していない。

### Host容量境界検証状況 (2026-07-14)

実時間のRaspberry Pi容量枯渇やSDカードへの不要な書き込み負荷を避け、オンディスクSQLiteの
`max_page_count`を初期DBより8 pageだけ大きく設定して、実際のCollector transactionを容量境界まで
繰り返した。テスト専用の製品failpointやSQLによるreading直挿入は使っていない。

- Siteをarchive-responsible target、cursor 0とした状態で、90 envelopeのreading、measurement outbox、
  ingest dedup claimをそれぞれ90件までatomicに保存した。次のenvelopeはSQLite容量不足により
  `SubmitError::NoAck`となった。
- 失敗後も3テーブルの件数は90/90/90で一致し、失敗したenvelopeの部分行やdedup claimは残らなかった。
  Site cursorは0のままで、DBは設定したpage上限を超えず、`PRAGMA quick_check = ok`だった。
- 上限を広げてCollectorを停止し、同じDBを再オープンしても90/90件のreading/outboxを保持していた。
  失敗した同一envelopeを再送すると受理され、reading、outbox、dedupは91/91/91へ回復した。
- 続けて通常のEdge、Mosquitto、Siteによる耐障害scriptを新規環境で再実行し、Edge/Site再起動、
  Broker再起動、Site単独停止をまたいで304 recordが欠番・重複なく再収束した。

これにより `BravePI Transmitter -> BLE Long Range -> BravePI Mainboard -> UART -> IoTKit Edge SQLite -> MQTT
-> MQTT Broker -> IoTKit Site raw SQLite -> accepted-through -> Edge cursor` は実機確認済みとなった。
平文MQTTは同一Piのloopbackだけを使う実験設定であり、実運用のTLS要件を緩和しない。
実機での下流停止中収集・UART復帰、hostでの決定的な容量境界、通常process群での再送収束を組み合わせ、
最初の実装ゲートは完了とする。Raspberry Piを実容量近くまで埋める長時間試験は行わない。接点入力は
UARTで0/1信号をdecode済みであり、必要時の軽い実信号確認に留める。

実験用Piでの初回native release buildは、空のbuild cacheからRust toolchainと依存crateを
最適化したため一時的にCPUを飽和させ、SSH応答も遅くなった。日常の実機反復はdebug buildを
使い、正式配置用release binaryは開発機またはCIでarm64向けに作ることを優先する。

### このゲートでは作らないもの

- BravePIのBLE、ペアリング、トランスミッタ管理、送信間隔/出力設定、Long Range到達距離の再検証
- BravePIへの取得要求、downlink command、接点出力
- YokaKit連携、UI、dashboard、Site projection、cloud/fleet管理
- 第三者デバイスingress、pairing、オンボーディングUI
- credential enrollment/rotation、multi-Edge運用、MQTT Broker HA
- Site backup/restore、archive repair、汎用fan-out
- calibration UI、local rule、通知、南向きcommand/DFU
- 汎用adapter SDK、adapter code generator、宣言型driver DSL

既に実装済みのHTTP ingress、control API、operation catalog等は削除しないが、このゲートの完了条件から外し、
必要な保守以外の機能追加を止める。既存のrpi-local/OPT3001経路も維持するが、このゲートの完了条件には
含めない。ゲート完了直後の実装スライスでは、Siteでcanonical sensor seriesへ当時の
`production_pulse`という意味をfuture-onlyで割り当て、`active_sample`または`active_edge`でsemantic
eventへ変換し、独立したMQTT exporterからapplicationへ配送する最小経路を作った。1 source seriesにつき
activeな意味は1つ、backfillは行わない。v1ではこの製品内部語を汎用的な「累積値」へ置き換え、
YokaKit Adapterだけが`kind=production`へ変換する。adapter templateやSDKはIoTKitの中心機能より後に置く。

### Site semantic sliceのhost live-broker検証 (2026-07-15)

Docker上のMosquitto 2.0.22とIoTKit Siteを一時環境で起動し、疑似Edge、Site、application subscriberを
別user・topic単位ACLで分離した。接点seriesへ`production_pulse / active_edge / active_value=1`を設定し、
次の順序でfuture-only境界からapplication配送までを確認した。

1. mapping作成前にpublication 1 (`0`)を受理し、mappingとMQTT routeを作成した。
2. publication 2 (`0`)はmapping後のbaselineとなり、semantic eventを生成しなかった。
3. publication 3 (`1`)でinactiveからactiveへのedgeを作り、`accepted-through=3`、
   `event_sequence=1`、`source_pub_seq=3`、`count=1`を確認した。
4. semantic eventは1行、QoS 1のMQTT outboxはpublish済み1行となり、application topicで同じ
   `event_id`のcontract v1 payloadを受信した。
5. publication 3を同一内容で再送してもraw record、semantic event、outboxの行数は3/1/1から増えなかった。

MQTTはat-least-onceであるため配送回数をexactly-onceとは扱わず、重複時も安定した`event_id`で識別する。
この検証はhost上のlive-broker happy path、mappingのfuture-only、`active_edge` baseline、QoS 1 publish、
同一publication再送の冪等性を対象とする。BravePIからraw保存までの経路は既に実機確認済みのため、
semantic pathをPiで重ねて確認することは完了条件にしない。

### クリーン導入commissioning smoke検証 (2026-07-15)

予約済みだったoptional `commissioning_smoke` familyを出口契約v1へ具体化し、公開CLIだけで合成レコードを
enqueueして配送状態を確認できるようにした。新規Edge DBを初期化し、通常のEdge、Mosquitto、SiteをDockerの
一時環境で起動した後、`iotkit-edgectl smoke enqueue`が返したepoch/pub_seqを`smoke status`へ渡した。
レコードは通常outboxとEdge固有topicを通ってSite raw SQLiteへ耐久保存され、同じtest_idをSite queryで確認し、
相関した`accepted-through`によってstatusが`pending`から`delivered`へ進んだ。テストスクリプトから
`publication_log`への直接INSERTと`target_registry`の直接cursor参照は除去した。物理センサー値、device登録、
検疫、semantic mapping、application eventはこの合成レコードに関与しない。

同日、broker停止中のsemantic MQTT outbox復旧もhost上で確認した。brokerを停止した状態でactive edgeの
raw recordをSite storeへ受理すると、semantic event 1行と`published_at IS NULL`のoutbox 1行が生成・保持された。
同じSite processを維持してbrokerを復旧すると、QoS 1でapplication subscriberへ同じ`event_id`が1回届き、
outboxのpendingは1件から0件へ収束した。mapping revision境界とroute作成前eventの非配送は今回再検証せず、
既存focused testの範囲に残す。

### 実運用形Site bootstrap検証 (2026-07-15)

EdgeをRaspberry Pi上のnative process、標準BrokerとSiteをLinux Site host上のDocker Composeとする導入経路を
追加した。`iotkit-edgectl mqtt-binding`の非secret JSON、明示したBroker hostname/bind address、operatorが
用意したserver certificate/key/CAから、匿名禁止設定、Edge Node単位のexact topic ACL、hash済みBroker
password database、Site credential、Edgeへ安全に引き渡すcredential/CA/TOML fragmentを生成する。
VPNや特定network製品、certificate発行はこのbootstrapの要件・責務に含めない。

一時的な新規Edge DBと自己署名test CAを使い、生成した構成で実際のMQTT TLS接続、Site raw耐久保存、
`accepted-through`、commissioning smokeの`delivered`までを確認した。また、余分なsecret fieldを含むbinding、
group/world readableなprivate key、hostnameとbind addressの不一致、certificate SAN不一致、Git repository内の
出力先を拒否し、失敗時に部分出力やsecret診断を残さないことを検査した。生成fileはすべて0600、directoryは
0700で、Composeのrender結果とargv/envへ平文credentialを展開しない。既存出力先の上書きも拒否するため、
credential rotationとin-place upgradeは明示的な別作業のままとする。

### BravePIとの責任境界

- センサー、トランスミッタ、BLE Long Range、ペアリング、メインボード上の端末管理はBravePIの責任とする。
- ペアリングは既存iOS applicationで行う。IoTKitにpairing UIや設定経路を作らない。
- ペアリング済み端末のデータはメインボードからUARTへ自動送出される前提とする。
- IoTKitの責任はUART streamの受信から始まり、frame decode、正規化、耐久保存、Siteへの引き渡しを担う。

以下のWave分割は長期ロードマップと既存決定の参照用に維持する。ゲート完了後は上記の次スライスを優先する。

## 決定1: 3段階Wave分割を採用

- **Wave 0「動く最小」**(目安1〜2ヶ月): 自社一号現場に置いて実データ収集を開始する。
  一号現場はプロセス内アダプタ2種(BravePI UART / I2C直結)のみ=第三者デバイス向け装備は実装しない。
- **Wave 1「他人に配れる」**: HTTP/MQTT ingress、トークン+TLS、オンボーディング/検疫UI、出口契約実装、
  操作カタログ+dry-run、desired/reported、スナップショット自動退避。
- **Wave 2「OSSとして公開」**: クライアントライブラリ3種+接続ガイド、A/B更新+OSイメージ、
  インシデントバンドル+AIハーネス統合、カメラsidecar、振動契約実装。

**読み替え規則**: 各文書の「第一波必須」は「**契約定義v1に含める**」と「**Wave 0で実装する**」の2つに分離して読む。
契約文書は最初から本番形で書き、実装だけを削る。

## 決定2: 契約の「凍結」を「安定意図(stable-intent)」に格下げ

最初の外部消費者または外部デバイス作者が現れるまで、D1の凍結リストは
「破壊的変更には移行ノートを書く」規律に緩める。買い手ゼロの時点での完全凍結は負債。

## 決定3: YokaKitは独立した参照consumerとする

当初の「YokaKit再設計を凍結する」という判断は、2026-07-17のYokaKit MQTT Purpose-Bound Signal
Contract合意により更新した。YokaKitはIoTKitに依存せず、合意済みMQTT契約を満たす任意の送信softwareから
入力を受ける。IoTKitもYokaKitをcore、Siteの汎用semantic model、adapter境界へ組み込まず、
YokaKit Output Adapterだけが同契約への変換を担う。

## 決定4: 「第ゼロ波」(レガシー環境でのAI診断実証)は見送り

価値は認めるが現実的でないと判断。柱3の価値検証は、Wave 1でAIハーネスを開発する際に
自社現場(Wave 0稼働中のEdge)に対して行う。

## 決定5: 制御プレーンの認証方式

**サーバー側TLS(自己署名+フィンガープリントピン留め)+ operatorトークン**に統一。
mTLSは不採用(取り込み面のper-device mTLS不採用、CA基盤を作らない決定と整合)。
ai-connectivity図のmTLS表記は本決定で上書き。
※**2026-07-12 Plan 6改訂**: D13由来のsetup-mode例外（未認証の閉集合）は廃止。
未所有中はnetwork API/UIをbindせず、local `iotkit-edgectl`で所有権確立後にoperatorトークン認証へ入る。

## 設計課題キュー(レビューで確定した優先順)

0. ~~BravePI device_number焼き込み/設定可能の実機・仕様確認~~ **解決(2026-07-02)**:
   実機確認は不可のため公式仕様書(BVPMB-01 Rev 1.3)の逆抽出分析で判断——**焼き込み固有値**
   =個体識別型hardware_idで確定(詳細はD5宿題節)。Wave 0ブロッカー解除
1. **series識別モデル**(最優先。dedupキー・認可・復旧・出口契約の根元。Wave 0のコード前に確定)
   → **確定候補: D5**
2. ~~ADR 42本(monojoh-authority)の生死棚卸し~~ **完了(2026-07-02)**: 全42本を判定
   (維持16/要改訂21/廃止1/保留4)。結果・権威規則・処置案は [../adr-inventory.md](../adr-inventory.md)。
   ADR本文への反映は各キュー確定時に処置案どおり実施(新規発見: 0031に安定意図段階と決定文書→ADR
   昇格ルールの穴)
3. 出口契約とADR 0028/0029/0030/0032/0035の統合方針(push/pull、アーカイブ責任消費者との接続)。
   0030は劣化契約(R17)とアーカイブ責任消費者ackを上位規則とし、no-silent-dropを
   「契約外の黙示破棄禁止」に読み替える。必須入力(D5波及): **検疫遷移の配送**(解除の再エンキュー・
   遡及検疫annotation=D5決定3)+**エポック複合カーソル**(D5決定3)
4. ~~測定レジストリの正本二義性の解消+初期語彙~~ **確定(2026-07-02)**:
   [D6](D6-measurement-registry.md)(二層+copy-on-enable+初期語彙10キー+構造化値型record)。
   **Wave 0の設計ブロッカーはこれで全解除**
5. ~~論点2: 南向き契約(コマンド/ヘルス/ライフサイクル)~~ **確定(2026-07-08)**:
   [D12](D12-southbound-contract.md)(世話のみ・権限3分類・DFUキャンペーン・受領側認証)。
   「取り込みのみ可」の読み替えは解除——形態③(外部アダプタ)は能力宣言による南向き任意参加(D12決定5)
6. ~~UIスコープ文書~~ **確定(2026-07-08、初期所有権は2026-07-12改訂)**: [D13](D13-ui-scope.md)(器と予算・未所有時local recovery+
   管理者パスフレーズ1本・画面在庫・NOTリスト・時間集約派生series予約)

## Waveマーキング(責務台帳への割当。実装計画時に精緻化)

- **Wave 0実装**: R1, R3, R6(初期語彙+検証最小), R7(台帳最小・CLI登録/承認+replace-hardware
  ガードレールCLI版=D5), R8, R11(範囲クエリ+CSV),
  R12(ヘルスJSON最小), R16, R17(retention+水位), R18(スキーマ列のみ・値はedge固定), R20, R22(手動export。
  スナップショット形式とエポック規則はD2 §3.5「R22最小契約」に従う)
- **境界の明文化(2026-07-02、外部レビュー指摘反映)**:
  - **R13**: Wave 0は**append-only監査イベント行(ledger_events)のみ**=R13の最小下地。
    R13本体(構造化イベント履歴・インシデントバンドル)はWave 1。D6の「自動有効化+R13監査イベント」は
    Wave 0ではこのイベント行への記録を意味する
  - **R9較正**: Wave 0では**較正は恒等(オフセット0/倍率1)で未実装**。R9本体はWave 1。
    ただし `value_semantics` 列・series値域検証(D6値域3層)・較正要再確認**状態の列**は初日から
    スキーマに存在する(後からの列追加でスキーマ破壊しない)
- **Wave 1実装**: R2, R9, R10, R13, R14, R15, R19, R21(手順化), R23, R4/R5(D12の範囲で確定:
  世話動詞・DFUキャンペーン・形態①②の受領経路。接点出力駆動は対象外=D12決定1)
- **Wave 2実装**: R21(A/B自動化), R5拡張, 振動/カメラ関連
