# D13: UI scope

Status: 確定、2026-07-13簡素化改訂

## Decision

最初のOPT3001 → Gateway → MQTT → Site実機縦切りにWeb UIを含めない。GatewayとSiteのCLI、health、
direct queryで収集・配送・保管責任の引き渡し・照会を検証する。

UIは既存のtyped operationとread modelの薄いclientであり、UIだけのmutation path、direct SQL、
秘密情報の再表示、保管責任cursor操作を作らない。

## Current operator surfaces

- Gatewayの構成・台帳・target操作: `iotkit-gatewayctl`
- Gatewayの状態確認: health JSONとCLI query
- Site raw archive確認: Site Serverのread-only CLI
- broker設定とstatic credential: Site operatorがGit外の設定fileで管理

credential、token、private keyをargv、URL、ログ、監査detail、query outputへ出さない。

## Application boundary

YokaKit UIはYokaKitの責務であり、IoTKit管理UIではない。設備、工程、生産、OEE、alarm、dashboardを
IoTKit UIへ取り込まない。IoTKitが将来提供するUIはGateway/Siteの設置、状態、generic observation、
契約上の操作だけを扱う。

## Deferred

- Gateway Web UIの拡張
- Site Console
- enrollment wizard、fleet operation、credential rotation UI
- charts、dashboards、notifications
- AI operator UI
- multi-Gateway aggregate health

これらはCLIによる実機journeyが成立し、反復されるoperator作業が観測されてから設計する。
