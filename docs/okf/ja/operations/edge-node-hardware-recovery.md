---
type: Runbook
title: "Edge Node hardware復旧クイックガイド"
description: "故障したEdge Node hostを交換するための現場判断表と印刷用チェックリストです。"
language: ja
translation_key: operations.edge-node-hardware-recovery
status: stable
revision: 1
---

# Edge Node hardware復旧クイックガイド

故障したEdge Node hostを交換するときに使用します。このページは現場チェックリストであり、
[導入と復旧 §8.1](installation-and-recovery.md#81-edge-nodeを暗号化backupから本番復帰する)の
正式commandや[Edge Node復旧契約](../contracts/edge-node-recovery-v1.md)を置き換えません。
詳細手順は最初から最後まで順番に実行し、一部のcommandだけを抜き出して使わないでください。

Scheduled backupは任意です。利用可能な暗号化backupとpassphraseの有無によって復旧pathを
選びます。

## 停止条件

次のいずれかに該当するときは交換candidateをfencedのままにして停止します。

- 旧hostが起動中、network接続中、または旧Broker credentialを利用可能である。
- 選択した暗号化artifactをescrow済みpassphraseでauthenticateできない。
- Broker、IoTKit Edge、candidateから必要なacknowledgementを得られない。
- Final reportが`state=completed`、`completion_acknowledged=true`、
  `cursor_converged=true`をすべて示していない。
- Local ownershipを再確立していない。
- Operatorにno-backup loss boundaryを受け入れる権限がない。

通信断は手順を遅らせることはあっても、fenceの迂回やnormal runtimeの起動を許可しません。
Credential、token、key、passphrase、hash、customer identifierをincident recordへ
記載しないでください。

## Pathを選ぶ

| 状況 | Path | Data boundary |
| --- | --- | --- |
| Authenticate済み暗号化backupとpassphraseがある | 「Backupあり」checklistと§8.1の全手順を実行する | Readingとdeduplication claimはauthenticated snapshot boundaryまでだけrestoreします。それより後のlocal tailは証明できない場合があります。 |
| 旧hostが健全で計画交換を行う | 最初に§7.1で暗号化backupを作成、authenticateし、off-hostに保持してから「Backupあり」へ進む | 新しくauthenticateしたsnapshotがrecovery boundaryになります。 |
| Authenticate済み暗号化backupまたはpassphraseがない | 「Backupなし」checklistを実行する | Readingとdeduplication claimはrestoreしません。これは明示的にloss boundaryを受け入れるclean replacementであり、restoreではありません。 |

Legacy snapshot、plaintext DB copy、SQL編集、自作handoffをbackupとして扱わないでください。

## 現場checklist：交換前

- [ ] Incident recordを作成し、復旧を統制する担当者を一人決める。
- [ ] Edge Node IDと最終状態の特定に必要なnon-secret evidenceを記録する。
- [ ] 旧hostを停止し、物理的に隔離する。
- [ ] 旧Broker credentialをfenceし、非secretのfence receiptを保持する。
- [ ] 旧hostとDBをincident evidenceとして保持し、消去、再利用しない。
- [ ] 実deploymentのruntime user/group、live DB path、deployment owner、
      supervisor unitを特定する。
- [ ] 上の表からpathを一つ選び、選択理由を記録する。

## 現場checklist：Backupあり

- [ ] Restore前に選択したartifactをauthenticateし、inspectする。
- [ ] [§8.1の本番復帰手順](installation-and-recovery.md#81-edge-nodeを暗号化backupから本番復帰する)を
      順番どおり最後まで実行し、新しいcandidate pathだけへrestoreする。
- [ ] IoTKit Edgeが正確なcandidateと新ledger epochをauthorizeするまでcandidateを
      fencedに保つ。
- [ ] `state=completed`、`completion_acknowledged=true`、
      `cursor_converged=true`を示すfinal reportを保持する。
- [ ] Local owner passphraseを対話的にresetする。Authenticated HTTP ingestを使う場合は、
      通常のtyped operationでdesired listener、TLS generation、device authorityを再適用する。
- [ ] `--replace-existing`で既存backup configurationをrecovered DBへrebindする。
- [ ] Fresh backupを作成、authenticateし、backup statusがhealthyであることを確認する。
      Off-hostに保持し、同じbackup IDのretained copyを再度authenticateする。
- [ ] 必要なgateをすべて通過した後だけ、deploymentのnormal runtimeを起動する。
- [ ] `remaining_gap_review_required`と明示的なpossible-loss boundaryをincident reviewへ記録する。
- [ ] 旧credentialまたは旧DBを再有効化せず、旧hostを廃止する。

## 現場checklist：Backupなし

- [ ] Authenticate済み暗号化backupと利用可能なpassphraseがないことを記録する。
- [ ] Readingとdeduplication claimをrestoreできないことを記録する。
- [ ] Clean replacementをcommissioningする前に、siteで必要なloss boundary承認を得る。
- [ ] 旧hostとstorageをevidenceとして保持する。後から検証済みsourceが見つかればincident判断を
      変更できる場合がある。
- [ ] 暗号化backup recovery flowを実行せず、SQLでcursorを変更せず、故障Nodeからの
      continuityを主張しない。
- [ ] Clean commissioningと後続のnew ledger epochを別operationとして計画、検証する。
      Downstream idempotencyでpossible duplicateを露出させ、restore済みcontinuityとは記載しない。

## Incident完了evidence

Backup recoveryでは、incident recordに次を揃えるまでcloseしません。

- 旧host隔離とBroker fence receipt
- 選択artifactのidentityとauthenticate成功結果
- Recovery IDとfinal report
- Local ownership再確立の証拠
- 復旧後のfresh backup ID、healthy status、authenticate済みoff-host copy
- Cursor convergence結果と明示的なremaining-gap判断

Backupなしreplacementでは、backup固有evidenceを捏造せずunavailableと記録し、承認済み
loss boundaryとclean commissioning evidenceを残します。このchecklistは印刷できますが、
secretは承認済みowner-only storageから出さないでください。
