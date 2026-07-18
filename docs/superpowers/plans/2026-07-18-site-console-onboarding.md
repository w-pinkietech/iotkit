# Site Console Onboarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新しく受信したデバイスを、実際の受信値とAdapter由来の情報を見ながら現場表示設定まで完了できる`/setup`導入フローを作る。

**Architecture:** raw recordとdescriptorを受信事実として保持し、SiteのSignal profile v2を独立した表示設定として追加する。storeは設定画面用のgrouped read modelを返し、typed application serviceがvalidationと候補導出を所有し、JSON APIとHTML Consoleが同じserviceを使う。

**Tech Stack:** Go 1.24、SQLite、`net/http`、`html/template`、埋め込みCSS/JavaScript、Go標準testing

## Global Constraints

- 設計正本は`docs/superpowers/specs/2026-07-18-site-console-operator-journey-design.md`。
- Edge canonical metadata、raw custody、semantic projection、output deliveryの挙動を変更しない。
- 全画面login必須。viewerは閲覧のみ、admin以上だけがprofileを変更できる。
- identifierはadmin以上の`/setup`表示とadmin限定APIだけへ出す。
- 全mutationは既存のCSRF、Origin、revision precondition、個人auditを維持する。
- Adapter由来の種類・単位は候補にだけ使い、保存済みprofileを上書きしない。
- `boolean`は`dimensionless`かつ`decimal_places=0`。数値の桁数は0以上6以下。
- 作業中は対象packageのtestを実行し、最終段階で`iotkit-site`の`go test ./...`を一度だけ実行する。
- ユーザーの指示があるまでcommit、push、PR作成をしない。

---

### Task 1: Signal Profile v2のdomainと永続化

**Files:**
- Modify: `iotkit-site/internal/siteapp/types.go`
- Modify: `iotkit-site/internal/store/migrations.go`
- Modify: `iotkit-site/internal/store/profiles.go`
- Modify: `iotkit-site/internal/store/profiles_test.go`
- Modify: `iotkit-site/internal/store/migrations_test.go`

**Interfaces:**
- Produces: `SignalProfileInput{DisplayName, DisplaySensorType, DisplaySensorTypeLabel, DisplayValueKind, DisplayUnitMode, DisplayUnit string; DecimalPlaces int}`
- Produces: 同じ表示項目と`Revision`、`UpdatedAt`を持つ`SignalProfile`
- Produces: `SignalProfileInput.Validate() error`と`SignalProfile.Complete() bool`

- [ ] **Step 1: profile validationの失敗testを書く**

`siteapp/service_test.go`へtable-driven testを追加し、unknown sensor type、custom labelなし、unknown value kind、booleanとunitの併用、numericのunit未入力、桁数-1/7を拒否する。`temperature/numeric/unit/°C/1`と`contact/boolean/dimensionless/empty/0`を許可する。

- [ ] **Step 2: validation testが未実装理由で失敗することを確認する**

Run: `go test ./internal/siteapp -run 'TestSignalProfileV2Validation' -count=1`

Expected: 新しいfieldまたはvalidationがないためFAIL。

- [ ] **Step 3: domain型とvalidationを最小実装する**

`SignalProfileInput.Validate`は表示名128 bytes、custom label64 bytes、unit32 bytes、閉じたsensor type/value kind/unit mode、組合せ制約を検証する。`SignalProfile.Complete`は同じ制約を満たすinputへ変換して判定する。

- [ ] **Step 4: domain testを通す**

Run: `go test ./internal/siteapp -run 'TestSignalProfileV2Validation|TestDispatchRoutesInventoryProfileOperations' -count=1`

Expected: PASS。

- [ ] **Step 5: migration保持testを書く**

version 9のDBへ旧`signal_profiles(display_name, revision)`を作成して値を入れ、最新migration後も表示名とrevisionが保持され、追加列が空のためprofileは未完了と読めるtestを追加する。

- [ ] **Step 6: migration testがversion 10未実装で失敗することを確認する**

Run: `go test ./internal/store -run 'TestSignalProfileV2Migration' -count=1`

Expected: 追加列がなくFAIL。

- [ ] **Step 7: schema migration version 10を追加する**

`signal_profiles`へ次の列を追加する。

```sql
display_sensor_type TEXT NOT NULL DEFAULT ''
display_sensor_type_label TEXT NOT NULL DEFAULT ''
display_value_kind TEXT NOT NULL DEFAULT ''
display_unit_mode TEXT NOT NULL DEFAULT ''
display_unit TEXT NOT NULL DEFAULT ''
decimal_places INTEGER NOT NULL DEFAULT 0 CHECK(decimal_places BETWEEN 0 AND 6)
```

- [ ] **Step 8: profile v2 CRUD/audit testを書く**

保存した全fieldが返り、一覧でも読め、更新時revisionが増え、audit summaryに秘密情報ではなく全表示設定が記録されることをtestする。

- [ ] **Step 9: CRUD testが旧SQLで失敗することを確認する**

Run: `go test ./internal/store -run 'TestUpdateSignalProfile' -count=1`

Expected: v2 fieldが保存されずFAIL。

- [ ] **Step 10: profile v2 SQLを実装する**

`INSERT ... ON CONFLICT DO UPDATE`と返却型、audit summaryを全fieldへ拡張する。入力文字列はvalidation後にtrimして保存する。

- [ ] **Step 11: Task 1の対象testを通す**

Run: `go test ./internal/siteapp ./internal/store -run 'SignalProfile|ProfileInput|ProfileOperations' -count=1`

Expected: PASS。

### Task 2: 登録待ちread modelと候補導出

**Files:**
- Modify: `iotkit-site/internal/siteapp/types.go`
- Create: `iotkit-site/internal/siteapp/setup.go`
- Create: `iotkit-site/internal/siteapp/setup_test.go`
- Modify: `iotkit-site/internal/siteapp/service.go`
- Modify: `iotkit-site/internal/siteapp/service_test.go`
- Modify: `iotkit-site/internal/store/inventory.go`
- Modify: `iotkit-site/internal/store/inventory_test.go`

**Interfaces:**
- Produces: `Repository.ListSetupDevices(context.Context, int) ([]SetupDeviceSource, error)`
- Produces: `Service.ListSetupDevices(context.Context, Actor, int) ([]SetupDevice, error)`
- Produces: `SetupDevice{Device DeviceSummary; Identifier *string; State SetupState; Signals []SetupSignal}`
- Produces: `SetupSignal{Signal SignalSummary; Profile SignalProfile; ProfileComplete bool; Candidate SignalProfileInput; CandidateMissing []string}`
- Consumes: Task 1の`SignalProfileInput`と`Complete`

- [ ] **Step 1: grouped store read modelの失敗testを書く**

同一deviceの2信号が一つにまとまり、identifier、channel、descriptor facts、current values、保存済みprofile v2が返るtestを追加する。通常inventory JSONにはidentifier/series keyが露出しない既存testも残す。

- [ ] **Step 2: store testのREDを確認する**

Run: `go test ./internal/store -run 'TestListSetupDevices' -count=1`

Expected: method/type未実装でFAIL。

- [ ] **Step 3: bounded queryでgrouping sourceを実装する**

deviceを最大100件取得し、signalを`device_ref`でgroup化する。identifierは`descriptor_devices.identifier`、channelは`descriptor_signals.channel_index`から読み、profile v2はdescriptor列と別fieldへscanする。

- [ ] **Step 4: grouped store testを通す**

Run: `go test ./internal/store -run 'TestListSetupDevices|TestListInventorySummariesJoinProfilesWithoutSourceIdentity' -count=1`

Expected: PASS。

- [ ] **Step 5: 候補変換と状態導出の失敗testを書く**

次をtestする。

- `temperature_c` → temperature/numeric/unit/descriptor unit
- `contact_state` → contact/boolean/dimensionless
- unknown key → customだがcustom label不足
- device profileなし → waiting_for_device
- device profileあり・profile不完全 → waiting_for_signal
- descriptor不足かつprofile不完全 → metadata_missing
- 全profile完成 → ready
- 保存済みprofileはdescriptor候補より優先される

- [ ] **Step 6: application testのREDを確認する**

Run: `go test ./internal/siteapp -run 'TestSetup' -count=1`

Expected: candidate/state導出未実装でFAIL。

- [ ] **Step 7: `setup.go`を実装する**

descriptor候補表、`SetupState`、`ListSetupDevices`を一つの責任にまとめる。actor validationとlimit 1..100を行い、候補だけではprofile完成扱いにしない。

- [ ] **Step 8: Task 2の対象testを通す**

Run: `go test ./internal/siteapp ./internal/store -run 'TestSetup|TestListSetupDevices|TestListInventory' -count=1`

Expected: PASS。

### Task 3: JSON APIとConsole導入画面

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/api_v1.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Modify: `iotkit-site/internal/sitehttp/static/console.js`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Produces: `GET /api/v1/setup/devices`（admin以上）
- Extends: `PUT /api/v1/signals/{signal_ref}/profile`
- Produces: `GET /setup`
- Extends: `POST /console/signals/{signal_ref}/profile` with `return_to=/setup`
- Consumes: Task 2の`Service.ListSetupDevices`

- [ ] **Step 1: API security/DTOの失敗testを書く**

anonymousは401、viewerは403、adminは200。admin responseはidentifier、descriptor facts、候補、profile完成状態を含むが、series key、system ID、raw payload全文を含まないことをtestする。profile PUTはv2 fieldを受理する。

- [ ] **Step 2: API testのREDを確認する**

Run: `go test ./internal/sitehttp -run 'TestSetupAPI|TestSignalProfileV2API' -count=1`

Expected: routeまたはfield未実装でFAIL。

- [ ] **Step 3: API routeとDTOを実装する**

`GET /api/v1/setup/devices`はadmin以上のread authorizationを行い、application read modelを専用DTOへ写す。identifierはこのDTOだけに含める。signal profile requestをv2へ拡張する。

- [ ] **Step 4: API testを通す**

Run: `go test ./internal/sitehttp -run 'TestSetupAPI|TestSignalProfileV2API' -count=1`

Expected: PASS。

- [ ] **Step 5: Console journeyの失敗testを書く**

login済みviewer/adminで`GET /setup`が表示でき、同じdeviceのsignalが一つのcardにまとまること、viewerにはidentifierとformがないこと、adminにはidentifierとformがあること、不足metadataの説明と生値が同時表示されることをtestする。profile保存後に`/setup?saved=1`へ戻り、monitor/logがprofileの名前・単位・桁数を使うこともtestする。

- [ ] **Step 6: Console testのREDを確認する**

Run: `go test ./internal/sitehttp -run 'TestSetupConsole|TestConsoleUsesSignalProfileV2' -count=1`

Expected: `/setup`が404または必要な表示がなくFAIL。

- [ ] **Step 7: Console controller/viewを実装する**

navigationへ「新しい機器」を追加する。`consoleData`へ`SetupDevices`と`SetupPendingCount`を追加し、statusとsetupが同じ完成判定を使う。statusの旧「意味付け未設定」は「登録待ち」へ変更し、semantic mapping有無と混同しない。

- [ ] **Step 8: `/setup` templateを実装する**

device cardごとに次を順番に表示する。

1. Edge、最終受信、descriptor状態、admin限定identifier
2. device名と場所
3. 各signalの生値、更新時刻、Adapter由来measurement/value type/unit/channel
4. Site現場設定の表示名、分類、custom分類名、値型、単位/単位なし、桁数
5. 不足項目、保存状態、次の行動

semantic変換formはこの画面へ複製せず、signal profile完了後に`/signals`へ進む導線だけ置く。

- [ ] **Step 9: CSS/JSを導入画面へ追加する**

desktopではdeviceの中にsignalsを縦に並べ、狭い画面では1 columnにする。`display_sensor_type=custom`時だけcustom labelを表示し、value kindがbooleanならunit modeをdimensionless、桁数を0へ固定する。JavaScriptが無くてもserver validationと全field表示で設定可能にする。

- [ ] **Step 10: effective表示をmonitor/logへ反映する**

未保存時はdescriptor候補、保存後はprofileの表示分類・単位・値型・桁数を使う。descriptor factsは上書きしない。桁数指定は数値表示だけに適用し、booleanはON/OFFを維持する。

- [ ] **Step 11: Task 3の対象testを通す**

Run: `go test ./internal/sitehttp -run 'TestSetup|TestSignalProfile|TestConsole|TestViewer' -count=1`

Expected: PASS。

### Task 4: Regression、実画面、文書整合

**Files:**
- Modify as needed: `docs/superpowers/specs/2026-07-18-site-console-operator-journey-design.md`
- Modify as needed: `/home/kenta/dev/iot/.runtime/iotkit-console-preview/capture-console.mjs`

**Interfaces:**
- Consumes: Tasks 1–3の完成したSite binaryとConsole
- Produces: desktop/mobileの実画面確認結果

- [ ] **Step 1: formatと静的差分を確認する**

Run: `gofmt -w internal/siteapp internal/store internal/sitehttp`

Run: `git diff --check`

Expected: formatting error、whitespace errorなし。

- [ ] **Step 2: Site全体testを一度だけ実行する**

Run: `go test ./...`

Expected: 全package PASS。raw acceptance、semantic projection、output delivery、auth regressionもPASS。

- [ ] **Step 3: preview binaryを更新して既存preview DBをmigrationする**

`go build`でpreview binaryを更新し、既存のsystemd preview serviceを再起動する。既存account、profile、raw dataが消えていないことを確認する。

- [ ] **Step 4: browserでdesktop/mobileを確認する**

実ブラウザで`/status`、`/setup`、`/monitor`、`/logs`を確認し、overflow、form label、keyboard focus、empty state、viewer/admin差、identifier非露出、保存後反映を確認する。

- [ ] **Step 5: 問題があればtestを先に追加して修正する**

見つけたbehavior defectごとに対象testをREDにし、最小修正、対象test再実行を行う。見た目だけの修正はbrowser captureのbefore/afterで確認する。

- [ ] **Step 6: 最終差分を自己レビューする**

Run: `git status --short`

Run: `git diff --stat`

Run: `rg -n 'FIXME|XXX|series_key|system_id' docs/superpowers/specs/2026-07-18-site-console-operator-journey-design.md iotkit-site/internal/sitehttp/templates/console.html`

Expected: 未解決の仮記述なし。内部identityは診断要件で認めた箇所以外の通常画面にない。commitは作られていない。
