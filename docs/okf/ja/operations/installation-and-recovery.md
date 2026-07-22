---
type: Runbook Overview
title: "IoTKit導入と復旧の概要"
description: "IoTKit Edge Node、Broker、IoTKit Edgeを安全に導入・確認・復旧するrunbookの入口です。"
language: ja
translation_key: operations.installation-and-recovery
status: stable
revision: 1
---

# IoTKit導入と復旧の概要

この文書は運用原則の概要です。実行するcommand、file mode、失敗時の停止点を含む正式手順は、同じGit revisionの`docs/edge-operations.md`を使用してください。

## 導入順序

1. Edge Nodeを初期化し、生成されたidentityと非secretのMQTT bindingを確認する。
2. IoTKit Edge hostで、Broker hostname、bind address、既存TLS証明書、秘密鍵、trust bundle、Edge Node bindingを使って導入資材を生成する。
3. Edge Nodeごとに分離されたcredential、ACL、CA trustを安全に配布する。secretをargv、環境変数、log、Gitへ置かない。
4. IoTKit Edgeの正本storage profileとして`embedded`または`postgres`を一つ選ぶ。起動時を含む全段階で別backendへfallbackせず、選択profileを開けなければ起動失敗または停止する。
5. local-only bootstrapで最初のsystem administratorを作り、Consoleへログインする。
6. Broker enrollmentがtransport接続だけを許可することを確認し、Edge NodeをConsoleで発見してexact incarnationのactivation request、Edge Nodeの適用、matching resultのcommitを完了する。その後のcommissioning smokeが`accepted-through`へ到達することを確認する。

BrokerとIoTKit Edgeは同じhostでも別hostでも構いません。DNS、LAN、firewall、VPN、証明書発行はdeployment責務です。ConsoleのHTTPS終端とMQTT BrokerのTLSは別の境界として管理します。

## 日常確認

Edge Node、Broker、IoTKit Edge、正本DB、証明書期限、pending custody、pending external output、backup結果を構造化statusで確認します。MQTT PUBACKだけをend-to-end成功と判断しません。

## 復旧原則

暗号化backupはidentity、cursor、設定、outboxを一貫した状態で含めます。復元は停止中の新しいpathまたは空のdatabaseへ行い、所有権、profile、schema、identity、cursorを検証してからtrafficを戻します。SQLiteからPostgreSQLへの変更もoffline migrationで行い、dual-writeしません。

認証情報を失った場合はhost上のlocal recovery operationを使います。network経由の未認証setup routeやHTTPへのfallbackは設けません。証明書は導入時に選択した発行元と更新clientで自動更新し、期限と更新失敗を監視します。
