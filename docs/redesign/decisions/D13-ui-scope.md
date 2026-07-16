# D13: UI scope

Status: 確定、2026-07-16 旧IoTKit operator capability継承方針追記

## Decision

最初のBravePI Transmitter → Long Range BLE → BravePI Mainboard UART → IoTKit Edge → MQTT Broker → IoTKit Site実機縦切りは、
Web UIを含めずに完了した。次のSite semantic sliceもCLIで設定・確認し、UIは含めない。

UIは既存のtyped operationとread modelの薄いclientであり、UIだけのmutation path、direct SQL、
秘密情報の再表示、保管責任cursor操作を作らない。

## Operator surfaces

- Edgeの構成・台帳・target操作: `iotkit-edgectl`
- Edgeの状態確認: health JSONとCLI query
- Site raw archive確認とsemantic mapping設定: IoTKit Site CLI
- Broker設定とstatic credential: Site operatorがGit外の設定fileで管理

credential、token、private keyをargv、URL、ログ、監査detail、query outputへ出さない。

## Application boundary

YokaKit UIはYokaKitの責務であり、IoTKit管理UIではない。品番、工程、生産、OEE、業務alarm、業務dashboardを
IoTKit UIへ取り込まない。IoTKitが提供するUIはEdge/Siteの設置、状態、generic observation、
契約上の操作に加え、Siteで保存済みseriesを`production_pulse`等の設定可能なセンサー意味へ対応付ける
routing・projection設定を扱う。

旧IoTKitが現場要求から獲得したoperator capabilityは、旧実装の肥大を理由に一律削除しない。現在値、
最近の変化、履歴検索、汎用データexport、storage状態、任意のcamera live viewは、業務dashboardではなく
センサー設置・収集確認・障害調査のためのIoTKit管理面として扱える。Site Consoleへ含める具体的な範囲は
Site Console/API設計で決め、取得・保存・queryを有界化する。

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
- Site Console
- enrollment wizard
- fleet operation、credential rotation UI
- 業務charts、業務dashboards、notifications（汎用monitor/logはSite Console設計で扱う）
- AI operator UI
- multi-Edge aggregate health

これらはCLIによる実機journeyが成立し、反復されるoperator作業が観測されてから設計する。
