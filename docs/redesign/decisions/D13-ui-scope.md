# D13: UI scope

Status: 確定、2026-07-18 Site Edge activation追記

## Decision

最初のBravePI Transmitter → Long Range BLE → BravePI Mainboard UART → IoTKit Edge → MQTT Broker → IoTKit Site実機縦切りは、
Web UIを含めずに完了した。次のSite semantic sliceもCLIで設定・確認し、UIは含めない。

UIは既存のtyped operationとread modelの薄いclientであり、UIだけのmutation path、direct SQL、
秘密情報の再表示、保管責任cursor操作を作らない。

## Operator surfaces

- Edgeの構成・台帳・target操作: `iotkit-edgectl`
- Edgeの状態確認: health JSONとCLI query
- Site raw archive確認とsemantic mapping設定: IoTKit Site CLI
- Broker設定、connection profile、static credential、certificate: 導入管理者が各Broker/Site/Edge hostの
  local CLIとGit外の所有者限定fileで管理

credential、token、private keyをargv、URL、ログ、監査detail、query outputへ出さない。

Site ConsoleはBroker connection profileを登録・編集・選択・切替しない。endpoint、TLS server name、CA、
credential、client ID、ACL、certificate更新方式はdeployment設定であり、Consoleは使用中profile名、接続状態、
実TLS接続で観測したcertificate期限と最終観測時刻、`接続確認済み`/`配送確認済み`/`要対応`の非秘密statusだけを
表示する。別host Broker内部の更新job成否は表示せず、Broker hostのlocal診断で扱う。
BrokerとSiteの同一host配置をUIの前提にしない。再設定とrollbackは対象hostのlocal CLIで行う。

Site Consoleはdescriptorで発見したEdgeを、deviceとは別階層で表示する。`接続設定済み`、`未登録`、
`登録処理中`、`登録済み`、`復旧確認待ち`を区別し、adminだけが初回Site activationを実行できる。
activationは既存Site application serviceのtyped operationを使い、Edge表示名、設置場所、exact ledger epoch、
操作状態、最終結果、登録前ローカル値が正式履歴へ入らないことを表示する。credential、ACL、endpoint、
certificateは表示用statusを除き操作しない。

## Adapter境界の表示

Input AdapterとOutput Adapterは左右対称の管理対象ではない。Input AdapterはEdge内部で物理機器固有の
通信をprovider-neutralなdevice、signal、measurementへ変換する実装境界であり、Site Consoleは
adapter type、instance、configured source、locator、bus addressを受け取らず、選択・設定もしない。
Consoleの入力側は「Edgeから受信した値」と表記し、Edge、デバイス、任意の表示用model ID、channel、
値形式、単位、生値、最終受信だけを表示する。model IDからAdapterやsemantic meaningを推測しない。

Output AdapterはSiteで確定したgeneric semantic observationを外部application向けtopic/payloadへ変換する
Site内部の純粋変換境界である。Consoleではruleごとにregistryが宣言する互換な変換形式を選べる。
Consoleの設定候補はregistry descriptorから作るが、同じbuildにversioned config presenterとPOST encoderが
あるAdapterだけを表示する。registryへAdapterを登録しただけで未実装の設定formを選択可能にしない。
Output Adapterの変換状態と、Site delivery layerが所有するMQTT接続、outbox、retry、PUBACK状態を別に
表示する。Broker endpoint、certificate、credential、ACLはどちらのAdapter設定にも混ぜない。

概要とセンサー詳細の共通導線は「受信した値 → Siteで作る値 → 外部へ送る」とする。センサー詳細を
一つのセンサーの入力、semantic rule、Output Routeを追える正本とし、各ruleから対象選択済みの外部出力
画面へ進める。専用のデータ経路画面や初回限定wizardを追加しない。

## Application boundary

YokaKit UIはYokaKitの責務であり、IoTKit管理UIではない。品番、工程、生産、OEE、業務alarm、業務dashboardを
IoTKit UIへ取り込まない。IoTKitが提供するUIはEdge/Siteの設置、状態、generic observation、
契約上の操作に加え、Siteで保存済みseriesを`production_pulse`等の設定可能なセンサー意味へ対応付ける
routing・projection設定を扱う。

旧IoTKitが現場要求から獲得したoperator capabilityは、旧実装の肥大を理由に一律削除しない。現在値、
最近の変化、履歴検索、汎用データexport、storage状態、任意のcamera live viewは、業務dashboardではなく
センサー設置・収集確認・障害調査のためのIoTKit管理面として扱える。Site Consoleへ含める具体的な範囲は
Site Console/API設計で決め、取得・保存・queryを有界化する。

v1のSite Consoleは、履歴検索、汎用CSV export、Site storage状態を必須とする。履歴はsensor、Edge、期間で
絞り込み、graphとtableを同じ検索条件から作る。高頻度・長期間のgraphは生record全件をbrowserへ送らず、
上限付きの時間bucket集約を使う。標準CSVは補正・判定・累積ruleを適用して永続化した
`semantic_observations_v3`を正本とし、時刻、Edge、sensor identity、rule名・kind、値、単位、series identityと
適用revisionを含める。export時に現在のruleで過去を再計算しない。`raw_records`のCSVは通信・設定調査用の
副導線として明示する。どちらも業務帳票やOutput Adapter固有payloadではない。検索やCSV取得は`viewer`も
利用できるが、保持期間変更、backup、復元、削除は管理権限とlocal host authorityを分離する。

storage画面はDB使用量、filesystem利用可能容量、保存件数、未配送outbox、最終検証済みbackupを事実として表示する。
単にHTTP応答中であることから「保存サービスは正常」と推測しない。sensor停止、Edge停止、Broker断、Site受信停止、
外部配送停止、容量警告を同じ「データが古い」表示へ潰さず、観測できない原因は「確認できません」と表示する。

camera映像はmeasurement/MQTT payloadへ載せず、Edge側のoptional media serviceからSiteのHTTPS originを
経由して表示する。MQTTはcameraの存在、能力、health等のversion付きmetadataにだけ利用する方向とし、
exact wire contractは別のFull lane設計まで固定しない。camera不在やmedia service停止はraw custodyを止めない。
初期版では外部application向けcamera stream、埋め込み用media API、cross-origin配信を提供しない。
将来の追加を妨げないようcamera identityとmedia serviceをSite ConsoleのHTMLから分離するが、未承認の
公開URL、認証方式、CORS/CSP契約を先に固定しない。

barcode等の文字列/離散観測は、日本の工場で広く使われるが、既存現場ではbarcode readerを受ける別systemが
MQTTでYokaKitへ直接送信している。IoTKitを全factory inputの必須中継点にせず、この既存経路を初期版の
責任範囲へ取り込まない。将来、IoTKitへreaderを直接接続する要求が生じた場合は、D7の予約どおりD1の
`Vec<f64>`を文字列/離散観測へ拡張し、取り込み契約v2と出口familyを同時に決める。数値へ偽装したり
YokaKit固有payloadをIoTKitへ持ち込んだりしない。barcodeを品番へ解釈する責務はYokaKit側に置く。

## Deferred

- Edge Web UIの拡張
- Broker接続変更・certificate/credential rotation UI
- deactivation/reactivation、Site transfer、legacy Edge adoption wizard
- fleet operation
- 業務charts、業務dashboards、notifications（汎用monitor/logはSite Console設計で扱う）
- AI operator UI
- multi-Edge aggregate health

これらはCLIによる実機journeyが成立し、反復されるoperator作業が観測されてから設計する。
