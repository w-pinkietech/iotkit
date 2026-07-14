# D13: UI scope

Status: 確定、2026-07-13簡素化改訂

## Decision

最初のBravePI Transmitter → Long Range BLE → BravePI Mainboard UART → IoTKit Edge → MQTT Broker → IoTKit Site実機縦切りに
Web UIを含めない。EdgeとSiteのCLI、health、direct queryで収集・配送・保管責任の引き渡し・照会を検証する。

UIは既存のtyped operationとread modelの薄いclientであり、UIだけのmutation path、direct SQL、
秘密情報の再表示、保管責任cursor操作を作らない。

## Current operator surfaces

- Edgeの構成・台帳・target操作: `iotkit-edgectl`
- Edgeの状態確認: health JSONとCLI query
- Site raw archive確認: IoTKit Siteのread-only CLI
- Broker設定とstatic credential: Site operatorがGit外の設定fileで管理

credential、token、private keyをargv、URL、ログ、監査detail、query outputへ出さない。

## Application boundary

YokaKit UIはYokaKitの責務であり、IoTKit管理UIではない。設備、工程、生産、OEE、alarm、dashboardを
IoTKit UIへ取り込まない。IoTKitが将来提供するUIはEdge/Siteの設置、状態、generic observation、
契約上の操作に加え、Siteで保存済みseriesを`production`等の設定可能なセンサー意味へ対応付ける
routing・projection設定を扱う。品番・工程master、生産実績、OEE、alarm、dashboardは扱わない。

## Deferred

- Edge Web UIの拡張
- Site Console
- enrollment wizard、fleet operation、credential rotation UI
- charts、dashboards、notifications
- AI operator UI
- multi-Edge aggregate health

これらはCLIによる実機journeyが成立し、反復されるoperator作業が観測されてから設計する。
