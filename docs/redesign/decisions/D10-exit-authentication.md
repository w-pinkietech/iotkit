# D10: 出口認証と経路

Status: 確定、2026-07-18 Site activation ACL追記

## 決定

Edgeは、配置環境が用意する到達可能なIP networkを通じて標準MQTT brokerへ接続する。経路は同一拠点LAN、
拠点間回線、VPN、routed WAN等のいずれでもよく、IoTKitはVPNの有無やTailscale等の特定製品を要件にしない。
VPNを使う場合も、それは到達経路とdefense in depthであり、MQTT application認証の代替ではない。

MVPの認証は次に限定する。

- brokerは匿名接続を禁止する。
- Edge Nodeごとにstatic credentialを1つ発行する。共有credentialを使わない。SiteにもEdgeとは別の
  Site固有credentialを発行し、全主体でcredentialを共有しない。
- usernameは`edge_node_id`へ束縛し、ACLは当該Edge Nodeのrecords/descriptors/activation result publishと
  accepted-through/activation request subscribeだけを許可する。Siteは全Edge Nodeの
  records/descriptors/activation result readとaccepted-through/activation request writeだけを持つ。
- brokerはTLSを使い、operatorが明示したinterface/addressだけへbindする。意図しないnetwork interfaceへ
  公開しない。
- credentialはargv、環境変数、ログ、Debug、監査detail、query outputへ出さない。
- credential fileはGit外の所有者限定fileとして配置する。

最初の1 Edge Node構成では、credentialの作成・配布・失効はoperatorが行う。自動enrollmentや自動rotationを
実装する前に、実機の収集・配送・再送・照会を完成させる。

## 配置と接続profile

BrokerとSiteの同一host配置はreference deploymentの一つであり、契約上の必須要件ではない。Broker、Site、
Edgeは別hostに配置でき、共有filesystemや同一Docker Compose projectを接続条件にしない。Mosquittoの
certificate/秘密鍵、listener、password database、ACLはBroker hostへ、Site固有のclient credentialはSite
hostへ、Edge Node固有のclient credentialは各Edge hostへ置く。

接続設定は次を一体として束縛するversion付きconnection profileで扱う。

- endpoint(hostname/IPとport)
- TLSで照合するserver name
- trust bundleへの固定参照
- 接続主体固有credentialへの固定参照
- MQTT client ID
- `edge_publish`、`site_ingest`、`site_external_publish`等のprincipal role

endpointだけを変更したり、以前のcredentialを別Brokerへ送ったりできる部分更新は許さない。profileを変える
場合は、導入管理者が所有者限定fileとlocal CLIで新しいprofileをinstallし、実際のclient hostから
`test -> activate`を行う。失敗時は直前のprofileへrollbackする。任意path、credential、CA、private key、
endpointの登録・変更・profile切替をSite Consoleから行わない。

Edgeは初回自己構成で`edge_node_id`を生成した後、Broker operatorがそのIDに束縛したcredential/ACLと
handoff bundleを生成する。初版はSite ConsoleからEdgeへ遠隔配布せず、各Edgeのlocal CLIでhandoffを
適用する。SiteはSite固有profileを別に受け取り、Brokerと同居していてもnetwork clientとして認証する。
Siteから外部Brokerへpublishする場合も、外部Broker用の別profileをSite hostへinstallする。

このBroker enrollmentは通信許可であり、Site raw historyへの参加許可ではない。Site activationは、既に
認証されたEdgeのexact ledger incarnationへ将来のrecords受理を許可する独立したapplication操作である。
Consoleはactivation IDとSite DB上のgrantを作るが、Broker credentialやACLを変更しない。

接続testはDNS、TCP、TLS chain/server name、MQTT authentication、CONNACKまでを確認する。EdgeからSiteへの
最終commissioningは、実recordがSiteへ耐久保存され`accepted-through`がEdgeへ返るまで確認する。Siteから
外部BrokerへのCONNACK成功はtopic publish ACLを証明しないため、Consoleでは`接続確認済み`と、最初のeventが
PUBACKされた`配送確認済み`を区別する。

## Broker証明書運用

Mosquitto server certificateの取得・検証・原子的切替・reload・新規TLS/MQTT probe・rollback・期限statusは、
IoTKitのtopic、semantic mapping、YokaKitに依存しないBroker運用componentがBroker host上で担う。IoTKitは
利用者、ACL、topic policyを所有し、Broker運用componentへIoTKit domain権威を渡さない。

初版の取得経路は、外部で完成済みのcertificate bundleを明示installする経路と、`lego`を使うACME経路に
分ける。ACME経路はPebbleで自動testし、特定の公開DNS provider対応は現場要求が決まるまで増やさない。
Linux production hostではOS非依存の一回実行commandをsystemd timerから起動する。Windows Site/Broker hostの
正式対応は後続とし、WSLは開発・評価用途に限定する。

通常のleaf certificate更新は、既にclientへ配ったtrust bundleで検証できる場合だけ自動activateする。
trust anchor/CA自体の変更は通常更新へ混ぜず、全clientへの新trust配布、移行確認、旧trust撤去を伴う別の
明示的commissioningとする。Broker運用componentは更新jobの成否と期限statusをBroker hostのlocal診断へ出す。
別hostのSite Consoleは、実TLS接続で観測したpeer certificateの期限、最終観測時刻、接続・配送状態だけを
非秘密のread modelとして表示し、Broker内部の更新成功を推測しない。初版は更新status専用の遠隔管理経路を
追加しない。Consoleはcertificate発行、upload、接続設定変更、profile切替を行わない。

## Authority

MQTT endpointを運営するBroker operatorがbroker credentialとACLの発行権威である。同一人物がSite operatorを
兼ねてもよいが、同一hostや同一管理componentを要件にしない。IoTKit Edgeは受け取った
credentialをtarget設定へ束縛し、別Edge Node namespaceへ使わない。

LAN、拠点間回線、VPN、名前解決、firewall等の到達経路は配置環境とsite operatorの責務である。VPNを使う
場合、そのprovider account、node admission、ACL、account recoveryもIoTKitの責務外とし、provider secretを
IoTKitへ保存しない。networkまたはVPN上のnode identityだけをEdge Node認証として扱わない。

## Failure behavior

- 認証失敗やACL違反ではpublicationをSite custodyとして扱わない。
- 認証に成功しても、Site activationが未完了またはledger epochが不一致ならrecordsを保存せずackしない。
- activation request/resultのPUBACK、retained descriptor、通常再接続はactivation完了やデータ削除を意味しない。
- credential失効後はbroker接続を拒否または切断する。
- brokerまたはnetwork経路の停止中もIoTKit Edgeはoutboxを保持する。
- credential喪失時はoperatorが新しいcredentialを発行し、Edge設定を更新する。データcursorはcredentialと独立して維持する。

## MVP security gate

server TLSと主体固有static credentialは、管理された工場LAN、小規模fleet、telemetry中心、Brokerの
internet非公開を前提にMVP baselineとして採用する。mTLSを延期する代わりに、production投入前に次を満たす。

- trust modeを`system roots`と`指定bundleのみ`に明示分離する。私設/固定CA profileでsystem rootsへ暗黙に
  追加して信頼範囲を広げない。TLS hostname検証を無効化しない。
- credentialの発行、安全なhandoff、install、test、activate、失効、再発行、廃止、rollbackをlocal CLIと
  手順で成立させる。失効時はpassword/ACL reloadだけで既接続clientが切断されると仮定せず、対象切断または
  Broker restartまで確認する。
- anonymous、誤password、別Edge namespace、過大権限、誤CA、誤hostname、期限切れcertificate、平文listenerを
  拒否するnegative integration testを持つ。secretがconfig render、argv、環境変数、log、errorへ出ないことも
  検査する。
- wire contractに比例したpacket/message、connection、inflight/queue、memory/disk上限と監視をproduction
  Broker profileへ置き、firewall/network segmentでも到達元を限定する。
- Mosquitto image/binaryを検証済みpatch versionまたはdigestへ固定し、更新手順を持つ。
- 平文credential handoffは所有者限定regular fileとして扱い、受領確認後の元bundle削除、backup除外または
  暗号化、紛失時失効を導入手順へ含める。

`scripts/test-mqtt-security.sh`は、匿名、誤password、別Edge namespace、Siteの過大権限、誤CA、誤hostname、
期限切れleaf certificate、平文接続、診断へのsecret漏洩を実Brokerに対して検査する。profile lifecycle、
credential失効、firewall、disk監視、certificate自動更新の完了証拠ではない。

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
- deactivation/reactivation、Site transfer、Edge Node ID reuse

mTLSは不採用として削除せず、次のいずれかが成立した時点で、hardware-bound keyを含む同等以上のdevice
authenticationと比較して必須化を再判断する。

- Brokerをinternet、第三者network、共有WANへ直接公開する。
- 第三者へ多数のEdgeを配布し、個別passwordの安全な配布・失効を運用で保証できない。
- 制御、安全性に関わるtopic、またはclone耐性を必要とする。
- 顧客、規制、接続先外部Brokerがclient certificateを要求する。

通常fileに置いたmTLS private keyは盗難・複製されたstatic passwordと同様にcloneでき、侵害済みEdge、Broker
侵害、過大ACL、DoSを解決しない。clone耐性が要件ならTPM/secure element等へのkey固定も要件に含める。
上記triggerがない間は、mTLS発行・更新・失効・CA rolloverをMVP実装計画へ展開しない。
