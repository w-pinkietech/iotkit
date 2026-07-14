# D10: 出口認証と経路

Status: 確定、2026-07-13 MVP簡素化改訂

## 決定

最初のEdge-to-Site実機スライスは、ユーザー管理のTailscale tailnet内で標準MQTT brokerへ接続する。
overlayは到達経路とdefense in depthであり、MQTT application認証の代替ではない。

MVPの認証は次に限定する。

- brokerは匿名接続を禁止する。
- Edge Nodeごとにstatic credentialを1つ発行する。共有credentialを使わない。
- usernameは`edge_node_id`へ束縛し、ACLは当該Edge Nodeのrecords publishとaccepted-through subscribeだけを許可する。
- brokerはTLSを使い、tailnet addressだけへbindする。
- credentialはargv、環境変数、ログ、Debug、監査detail、query outputへ出さない。
- credential fileはGit外の所有者限定fileとして配置する。

最初の1 Edge Node構成では、credentialの作成・配布・失効はoperatorが行う。自動enrollmentや自動rotationを
実装する前に、実機の収集・配送・再送・照会を完成させる。

## Authority

MQTT endpointを運営するSite operatorがbroker credentialとACLの発行権威である。IoTKit Edgeは受け取った
credentialをtarget設定へ束縛し、別Edge Node namespaceへ使わない。

Tailscale account、node admission、ACL、account recoveryはoverlay providerとsite operatorの責務であり、
IoTKitはprovider secretを保存しない。Tailscale nodeであることだけをEdge Node認証として扱わない。

## Failure behavior

- 認証失敗やACL違反ではpublicationをSite custodyとして扱わない。
- credential失効後はbroker接続を拒否または切断する。
- brokerまたはoverlay停止中もIoTKit Edgeはoutboxを保持する。
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
