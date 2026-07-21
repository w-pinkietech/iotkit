# IoTKit Edge storage capacity regression smoke

IoTKitは、`embedded`または`postgres`という名前だけで無制限の規模を保証しない。
対応規模は、対象version、hardware、payload、rule、保持期間、backup/query負荷を固定した
再現可能な計測結果で判断する。

```bash
scripts/test-edge-capacity.sh /secure/report/directory
```

このsmokeは両profileへ同じ4 Edge Nodes、各8 sensor、合計8,000 raw recordを投入し、通常batch受理、
最大8,000件の履歴読出し、暗号化backupを順に行う。JSON reportにはprofile、records/s、batch受理p99、
query/backup時間、DB bytes、pending output、projection failureを残す。reportを保存していない構成を
「検証済み規模」として案内してはならない。

この短いsmokeは回帰検知用であり、実導入のsizingや対応上限を証明しない。本格導入前には予定する
Edge Node数、sensor数、ピークrecords/s、平均payload、意味付けrule数、保持日数、CSV/graph利用、
外部Broker停止、backup、restartを再現し、少なくとも次を記録する。

- accepted-through p99と未ack backlog
- 意味付けprojectionとoutput outboxの遅延
- DB/WAL容量、空き容量、増加量/日
- CPU、RAM、履歴query時間、100,000件CSV時間、backup時間
- 強制終了後の再起動時間とcursor/hash整合性

SQLiteで上限を超える場合は、同一IoTKit Edgeを停止移行してPostgreSQL profileへ切り替える。
SQLiteとPostgreSQLを同時に正本にしたり、TimescaleDBへrawを二重保存したりしない。
