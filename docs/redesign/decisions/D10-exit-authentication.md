# D10: 出口認証と経路

Status: 確定、2026-07-15 network方式非依存へ改訂

## 決定

Edgeは、配置環境が用意する到達可能なIP networkを通じて標準MQTT brokerへ接続する。経路は同一拠点LAN、
拠点間回線、VPN、routed WAN等のいずれでもよく、IoTKitはVPNの有無やTailscale等の特定製品を要件にしない。
VPNを使う場合も、それは到達経路とdefense in depthであり、MQTT application認証の代替ではない。

MVPの認証は次に限定する。

- brokerは匿名接続を禁止する。
- Edge Nodeごとにstatic credentialを1つ発行する。共有credentialを使わない。
- usernameは`edge_node_id`へ束縛し、ACLは当該Edge Nodeのrecords publishとaccepted-through subscribeだけを許可する。
- brokerはTLSを使い、operatorが明示したinterface/addressだけへbindする。意図しないnetwork interfaceへ
  公開しない。
- credentialはargv、環境変数、ログ、Debug、監査detail、query outputへ出さない。
- credential fileはGit外の所有者限定fileとして配置する。

最初の1 Edge Node構成では、credentialの作成・配布・失効はoperatorが行う。自動enrollmentや自動rotationを
実装する前に、実機の収集・配送・再送・照会を完成させる。

## Authority

MQTT endpointを運営するSite operatorがbroker credentialとACLの発行権威である。IoTKit Edgeは受け取った
credentialをtarget設定へ束縛し、別Edge Node namespaceへ使わない。

LAN、拠点間回線、VPN、名前解決、firewall等の到達経路は配置環境とsite operatorの責務である。VPNを使う
場合、そのprovider account、node admission、ACL、account recoveryもIoTKitの責務外とし、provider secretを
IoTKitへ保存しない。networkまたはVPN上のnode identityだけをEdge Node認証として扱わない。

## Failure behavior

- 認証失敗やACL違反ではpublicationをSite custodyとして扱わない。
- credential失効後はbroker接続を拒否または切断する。
- brokerまたはnetwork経路の停止中もIoTKit Edgeはoutboxを保持する。
- credential喪失時はoperatorが新しいcredentialを発行し、Edge設定を更新する。データcursorはcredentialと独立して維持する。

## Deferred hardening

以下はMVPの完了条件ではない。

- one-use enrollment ticket
- box-key mTLS enrollment
- short-lived credentialとtwo-slot rotation
- unattended re-issuance
- credential clock rollback high-water
- clone detectionとgeneration anchor
- Site backup/restore時のcredential reconciliation
- fleet enrollment、canary rotation、decommission automation

複数Edge Nodeを第三者へ配布する段階で、実際の設置journeyと脅威モデルを基に別途判断する。それまでは
この一覧を実装計画へ展開しない。
