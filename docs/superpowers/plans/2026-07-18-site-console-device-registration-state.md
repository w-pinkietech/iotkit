# Site Console Device Registration State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** デバイス情報の保存後、未設定センサーが残っていてもデバイスを「登録待ち」と数えない。

**Architecture:** `SetupDevice.State`は画面カード全体の次の作業を示す既存の集約状態として維持する。Consoleの`SetupPendingCount`だけを`SetupWaitingForDevice`に限定し、センサー未設定は既存の`ProfileComplete`とカード状態で表示する。

**Tech Stack:** Go 1.24、`html/template`、Go標準testing

## Global Constraints

- `device`は「デバイス」、`signal`は「センサー」と表示する。
- デバイス登録待ちはdevice profile未保存だけを意味する。
- signal profile未保存をデバイス登録待ちへ加算しない。
- 新しいDB columnや完了flagを追加しない。
- ユーザーの指示があるまでcommit、push、PR作成をしない。

---

### Task 1: Registration pending count

**Files:**
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `siteapp.SetupDevice.State`と`siteapp.SetupWaitingForDevice`
- Produces: device profile未保存だけを数える`consoleData.SetupPendingCount`

- [ ] **Step 1: Write the failing regression test**

`TestDeviceStopsBeingRegistrationPendingAfterDeviceProfileSave`で、device profile保存済みかつsignal profile未保存のfixtureを作る。`/setup`と`/status`のHTMLが「0台のデバイスが登録待ち」を示し、「新しいデバイスが見つかりました」を示さないことを検証する。

- [ ] **Step 2: Run the regression test and verify RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-next-go-build go test ./internal/sitehttp -run TestDeviceStopsBeingRegistrationPendingAfterDeviceProfileSave -count=1
```

Expected: `1台のデバイスが登録待ち`のためFAILする。

- [ ] **Step 3: Restrict the pending count**

`/setup`と`/status`の集計条件を次へ変更する。

```go
if device.State == siteapp.SetupWaitingForDevice {
	data.SetupPendingCount++
}
```

- [ ] **Step 4: Verify GREEN and regressions**

Run:

```bash
env GOCACHE=/tmp/iotkit-next-go-build go test ./internal/sitehttp -count=1
env GOCACHE=/tmp/iotkit-next-go-build go test ./... -count=1
git diff --check
```

Expected: 全commandがexit code 0。

- [ ] **Step 5: Verify the browser journey**

Preview binaryを更新し、device profile保存済み・signal profile未保存の画面で、デバイス登録待ちが0、カードが「センサーを設定」、センサーが「確認して保存」と表示されることを確認する。
