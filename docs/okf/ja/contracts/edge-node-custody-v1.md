---
type: Contract Overview
title: "Edge Node保管責任契約 v1の概要"
description: "Edge NodeからIoTKit Edgeへraw recordの保管責任を移すMQTT契約の入口です。"
language: ja
translation_key: contracts.edge-node-custody-v1
status: stable
revision: 1
---

# Edge Node保管責任契約 v1の概要

この文書は境界を理解するための概要です。record schema、topic、cursor、ack、検証順序の正式契約は、同じGit revisionの`docs/exit-contract.md`と共有egress fixture、対応するRust/Go validatorの契約成果物setにあります。

この契約は、Edge Nodeが保持するraw recordを標準MQTT Broker経由でIoTKit Edgeへat-least-once配送し、耐久保存の責任を明示的に引き渡す境界です。YokaKitなどの外部アプリケーション向け契約ではありません。

各Edge Node incarnationは`edge_node_id`とledger epochでfenceされ、公開可能なrecordを単調増加するpublication sequenceへ割り当てます。global identityは`(edge_node_id, ledger_epoch, pub_seq)`です。同じcontent fingerprintを持つexact replayだけを冪等に受理します。同一identityで内容が異なる場合はcustody conflictとしてbatch全体を拒否し、`accepted-through`を返しません。

Broker enrollmentはcredentialとACLによるtransport接続許可だけです。custody streamの開始権限ではありません。activationは三段階です。Console操作はexact `(edge_node_id, ledger_epoch)`のrequestをIoTKit Edgeへ耐久enqueueします。Edge Nodeがそのrequestを検証・耐久適用して境界を固定した後にだけ、将来のrecordへpublication admissionを開きます。IoTKit Edgeはmatching resultをcommitしてincarnationをactiveにした後だけrecordを保存・ackします。

IoTKit Edgeはbatch全体を検証し、raw recordと連続cursorを同じDB transactionでcommitします。その後にだけ、batch境界とincarnationが一致するapplication-level `accepted-through`を返します。MQTT PUBACK、message受信、検証開始はcustody移転を意味しません。

Edge Nodeは`accepted-through`より先のoriginalを保持します。通常の保持処理で削除できるのは、契約上purge可能になった範囲だけです。未ack originalの削除は最終的な明示data-loss段階であり、監査とgap記録なしに行ってはいけません。

activation前の観測はpublication sequenceを持たず、後から再送しません。quarantine中のrecordもoutboxを持ちません。解除後に配送する場合は、現在のactivationとpublication admissionを耐久transactionで通過する必要があります。
