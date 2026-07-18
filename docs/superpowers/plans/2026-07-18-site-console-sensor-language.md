# Site Console Sensor Language Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consoleのsignal表記を「センサー」へ変更し、deviceは「デバイス」として区別する。

**Architecture:** store、application service、JSON API、URLは変更せず、Console controller、view model、templateの表示語彙だけを変更する。device profileはデバイス名・設置場所、signal profileはセンサー名・種類・単位を担当し、signalの現在値はセンサーの属性として一行で表示する。

**Tech Stack:** Go 1.24、`html/template`、埋め込みCSS/JavaScript、Go標準testing

## Global Constraints

- 設計正本は`docs/superpowers/specs/2026-07-18-site-console-operator-journey-design.md`の5.1節。
- internal type、DB schema、API field、URLは改名しない。
- 通常Consoleでdeviceは「デバイス」、signalは「センサー」と表示する。
- measurement key、channel、value typeはAdapter由来の詳細情報として残す。
- 一つのdeviceに複数signalがある場合、一つのデバイス配下に複数のセンサーとして表示する。
- monitorとhistoryではセンサー名と現在値を一行にまとめ、「センサー」「値」を別列のentityにしない。
- `/signals`ではprofile編集を重複させず、semantic definitionを「センサー設定」として扱う。
- ユーザーの指示があるまでcommit、push、PR作成をしない。

### Task 1: Operator vocabulary contract

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server_test.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`

- [ ] Console page testへ`/monitor`=`センサーの現在値`、`/setup`=`デバイス管理`、`/signals`=`センサー設定`を追加する。
- [ ] rendered HTMLに利用者向けの「信号」がなく、deviceが「デバイス」と表示されることをtestする。
- [ ] testを実行して誤ったdevice→センサー表記で失敗することを確認する。
- [ ] page title、description、fallback name、setup state label、audit labelを正しい用語へ変更する。
- [ ] 対象testを通す。

### Task 2: Navigation and page content

**Files:**
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

- [ ] navigationが「センサー」「デバイス管理」「センサー設定」を持つtestを書く。
- [ ] setup pageが物理groupを「デバイス」、配下signalを「センサー」と表示するtestを書く。
- [ ] monitorとhistoryが一つのsignalを一つのセンサー行として表示するtestを書く。
- [ ] testの失敗を確認する。
- [ ] template全体の利用者向け文言を現場語へ置換する。
- [ ] `/devices`はnavigationから外し、`/signals`はprofile formを外してsemantic formだけを「センサー設定」として表示する。
- [ ] 対象testを通す。

### Task 3: Browser and regression verification

**Files:**
- Modify: `/home/kenta/dev/iot/.runtime/iotkit-console-preview/capture-console.mjs`

- [ ] browser captureの期待値を新navigationへ更新する。
- [ ] `gofmt`と`git diff --check`を実行する。
- [ ] `go test ./...`を実行する。
- [ ] preview binaryを更新し、Tailscale previewを再起動する。
- [ ] desktop/mobileで`/status`、`/monitor`、`/setup`、`/signals`をcaptureする。
- [ ] rendered bodyとscreenshotを自己レビューし、通常画面に内部語が残らないことを確認する。
