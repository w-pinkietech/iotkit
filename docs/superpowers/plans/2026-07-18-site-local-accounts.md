# IoTKit Site Local Accounts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Site内蔵の個人account、3段階のrole、browser session、local CLI復旧を実装し、Site Console/APIをlogin必須にする。

**Architecture:** SQLiteはaccount、session、個人単位auditを保持し、password hashとsession token原文を外へ出さない。HTTPとCLIは共通のtyped Site application serviceを使い、HTTP middlewareが認証済みprincipalとroleを各handlerへ渡す。

**Tech Stack:** Go 1.25、SQLite (`modernc.org/sqlite`)、Argon2id (`golang.org/x/crypto/argon2`)、`net/http`、server-rendered HTML。

## Global Constraints

- 初期版はlocal accountのみ。Keycloak/OIDC連携は実装しない。
- roleは`viewer`、`admin`、`system_admin`の3種類。
- passwordは12〜128文字、定期変更と文字種規則なし。
- sessionはidle 8時間、absolute 24時間、永続loginなし。
- password、password hash、session token、credential、private keyをresponse、log、audit、Gitへ出さない。
- accountは削除せず無効化し、login IDを再利用しない。
- 最後の有効なsystem adminを無効化・降格しない。
- Site Console/APIはlogin必須。未認証公開はlogin assetと内容なしhealthだけ。

---

### Task 1: Accountとsessionの永続モデル

**Files:**
- Modify: `iotkit-site/internal/store/migrations.go`
- Create: `iotkit-site/internal/store/accounts.go`
- Create: `iotkit-site/internal/store/accounts_test.go`
- Modify: `iotkit-site/internal/store/migrations_test.go`
- Modify: `iotkit-site/internal/store/audit.go`
- Modify: `iotkit-site/internal/store/audit_test.go`

**Interfaces:**
- Produces: `CreateAccount`, `GetAccountByLoginID`, `ListAccounts`, `UpdateAccount`, `ReplacePassword`, `CreateSession`, `GetSessionByTokenHash`, `TouchSession`, `RevokeSession`, `RevokeAccountSessions`。
- Produces: account audit actor `account`と操作時点のlogin ID/表示名snapshot。

- [ ] **Step 1: migration testを先に追加する**

schema version 5へのupgradeで`site_accounts`と`site_sessions`が作られ、既存raw/audit rowが残り、同じ
`login_id_normalized`を再登録できないtestを書く。

- [ ] **Step 2: testが未実装で失敗することを確認する**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/store -run 'TestMigrations|TestAccount'`
Expected: FAIL（schema/tableまたはrepository methodが存在しない）。

- [ ] **Step 3: migrationとrepositoryを最小実装する**

`site_accounts`には`account_ref`、`login_id`、`login_id_normalized`、`display_name`、`password_phc`、
`role`、`state`、`must_change_password`、timestampを保存する。`site_sessions`には`session_ref`、
SHA-256 token hash、account ref、idle/absolute expiry、last seen、revocationを保存する。passwordとtoken原文を
引数以外のstruct、error、audit summaryへ含めない。

- [ ] **Step 4: store testを通す**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/store`
Expected: PASS。

- [ ] **Step 5: commitする**

```bash
git add iotkit-site/internal/store
git commit -m "feat(site): persist local accounts and sessions"
```

### Task 2: Password、認証、role付きtyped operation

**Files:**
- Create: `iotkit-site/internal/siteauth/password.go`
- Create: `iotkit-site/internal/siteauth/password_test.go`
- Create: `iotkit-site/internal/siteauth/service.go`
- Create: `iotkit-site/internal/siteauth/service_test.go`
- Modify: `iotkit-site/internal/siteapp/types.go`
- Modify: `iotkit-site/internal/siteapp/service.go`
- Modify: `iotkit-site/internal/siteapp/service_test.go`

**Interfaces:**
- Produces: `siteauth.HashPassword(string) (string, error)`と`siteauth.VerifyPassword(string, string) (bool, bool, error)`。
- Produces: `siteauth.Login(ctx, loginID, password, sourceRef) (IssuedSession, error)`。
- Produces: `siteapp.Principal{AccountRef, LoginID, DisplayName, Role}`。
- Produces: typed operations `CreateAccount`、`ChangeAccount`、`ResetAccountPassword`、`ChangeOwnPassword`。

- [ ] **Step 1: security behavior testを先に追加する**

Argon2id parameter保存、wrong passwordの同一error、初回変更強制、3 role認可、最後のsystem admin保護、
account変更時の全session失効をtable-driven testにする。

- [ ] **Step 2: testが失敗することを確認する**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/siteauth ./internal/siteapp`
Expected: FAIL（未定義type/function）。

- [ ] **Step 3: passwordと認証serviceを実装する**

Argon2idはmemory 64 MiB、iterations 3、parallelism 1、salt 16 byte、hash 32 byteをPHC形式で保存する。
random生成は`crypto/rand`、比較は`subtle.ConstantTimeCompare`を使う。検証同時数2のsemaphoreと有界待ちを持つ。

- [ ] **Step 4: typed operationとrole認可を実装する**

全account mutationを`siteapp.Service.Dispatch`へ追加する。`viewer`はreadのみ、`admin`は設定mutation、
`system_admin`だけaccount mutationを許可する。local CLI principalはbootstrap/recovery operationだけ許可する。

- [ ] **Step 5: focused testを通す**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/siteauth ./internal/siteapp`
Expected: PASS。

- [ ] **Step 6: commitする**

```bash
git add iotkit-site/internal/siteauth iotkit-site/internal/siteapp iotkit-site/go.mod iotkit-site/go.sum
git commit -m "feat(site): add local account authentication"
```

### Task 3: 初期所有権と緊急復旧CLI

**Files:**
- Modify: `iotkit-site/cmd/iotkit-site/main.go`
- Modify: `iotkit-site/cmd/iotkit-site/main_test.go`

**Interfaces:**
- Produces: `iotkit-site account bootstrap --db ... --login-id ... --display-name ...`。
- Produces: `iotkit-site account reset-password --db ... --login-id ...`。
- Produces: `iotkit-site account recover-system-admin --db ... --login-id ... --display-name ...`。

- [ ] **Step 1: CLI testを先に追加する**

passwordがargvに存在しないこと、TTYで非echo入力すること、owner-only password fileを明示指定できること、
bootstrap二重実行拒否、reset/recoveryでsessionが失効しauditが残ることをtestする。

- [ ] **Step 2: testが失敗することを確認する**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./cmd/iotkit-site -run Account`
Expected: FAIL（unknown command）。

- [ ] **Step 3: account CLIを実装する**

CLIはtyped application serviceを呼び、password、hash、tokenをstdout/stderrへ出さない。password fileはregular
fileかつgroup/other permissionなしを要求する。初回system adminが無い状態でnetwork account作成は行わない。

- [ ] **Step 4: focused testを通す**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./cmd/iotkit-site -run Account`
Expected: PASS。

- [ ] **Step 5: commitする**

```bash
git add iotkit-site/cmd/iotkit-site
git commit -m "feat(site): add local account recovery commands"
```

### Task 4: Login必須HTTP API

**Files:**
- Create: `iotkit-site/internal/sitehttp/server.go`
- Create: `iotkit-site/internal/sitehttp/session.go`
- Create: `iotkit-site/internal/sitehttp/middleware.go`
- Create: `iotkit-site/internal/sitehttp/session_test.go`
- Create: `iotkit-site/internal/sitehttp/middleware_test.go`
- Create: `iotkit-site/internal/sitehttp/accounts.go`
- Create: `iotkit-site/internal/sitehttp/accounts_test.go`
- Modify: `iotkit-site/cmd/iotkit-site/main.go`

**Interfaces:**
- Produces: `POST/GET/DELETE /api/v1/session`、`POST /api/v1/password`。
- Produces: system admin限定account list/create/update/reset API。
- Produces: `RequireRole(viewer|admin|system_admin)` middleware。

- [ ] **Step 1: handler security testを先に追加する**

全readの401、role別403、generic login failure、Secure/HttpOnly/SameSite cookie、idle 8h/absolute 24h、
logout/password変更時失効、CSRF、Origin、rate limit、`Cache-Control: no-store`をtestする。

- [ ] **Step 2: testが失敗することを確認する**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/sitehttp`
Expected: FAIL（package/handler未実装）。

- [ ] **Step 3: session endpointとmiddlewareを実装する**

session tokenは32 byte random、DBへSHA-256 hashだけ保存する。CSRF tokenも原文をDBへ保存しない。
未認証公開routeはlogin assetと内容なしhealthだけをallowlistし、他routeはdefault denyにする。

- [ ] **Step 4: account APIをtyped operationへ接続する**

HTTP handlerからStoreへ直接書かない。account responseからpassword hash、session hash、内部security fieldを除外する。

- [ ] **Step 5: focused testを通す**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/sitehttp ./cmd/iotkit-site`
Expected: PASS。

- [ ] **Step 6: commitする**

```bash
git add iotkit-site/internal/sitehttp iotkit-site/cmd/iotkit-site
git commit -m "feat(site): require authenticated console sessions"
```

### Task 5: Login・初回変更・account管理画面

**Files:**
- Create: `iotkit-site/internal/sitehttp/templates/login.html`
- Create: `iotkit-site/internal/sitehttp/templates/change-password.html`
- Create: `iotkit-site/internal/sitehttp/templates/accounts.html`
- Create: `iotkit-site/internal/sitehttp/assets/site.css`
- Create: `iotkit-site/internal/sitehttp/console_test.go`
- Modify: `iotkit-site/internal/sitehttp/server.go`

**Interfaces:**
- Consumes: Task 4のsession/account API。
- Produces: login、初回password変更、system admin用account管理journey。

- [ ] **Step 1: HTML journey testを先に追加する**

未認証redirect、初回変更まで他画面を使えないこと、viewer/adminにaccount操作が見えないこと、無効化と
password resetの確認表示、内部ID/secret非表示、keyboard操作可能なlabel/buttonをtestする。

- [ ] **Step 2: testが失敗することを確認する**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/sitehttp -run Console`
Expected: FAIL（template未実装）。

- [ ] **Step 3: server-rendered画面を実装する**

既存brand asset方針に合わせ、外部CDNとNode.js buildを使わない。system admin画面では削除ではなく
無効化と表示し、最後のsystem adminを変更できない理由を日本語で示す。

- [ ] **Step 4: account sliceの全testを通す**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./internal/store ./internal/siteauth ./internal/siteapp ./internal/sitehttp ./cmd/iotkit-site`
Expected: PASS。

- [ ] **Step 5: 正本との整合を確認する**

Run: `rg -n 'settings-session|共有settings passphrase|匿名read surface' docs iotkit-site`
Expected: 現行仕様・実装に旧方式の参照なし（履歴説明を除く）。

- [ ] **Step 6: commitする**

```bash
git add iotkit-site/internal/sitehttp docs/superpowers/specs/2026-07-15-site-console-api-design.md
git commit -m "feat(site): add local account console"
```

### Task 6: 最終検証

**Files:**
- Modify only when verification finds an account-slice defect.

**Interfaces:**
- Produces: PR前のfresh verification evidence。

- [ ] **Step 1: Go全packageを一度実行する**

Run: `env GOCACHE=/home/kenta/dev/iot/.cache/iotkit-go-build GOMODCACHE=/home/kenta/dev/iot/.cache/iotkit-go-mod go test ./...`
Expected: PASS。

- [ ] **Step 2: repository全体gateを一度実行する**

Run: `scripts/verify.sh`
Expected: PASS。

- [ ] **Step 3: secret漏えいと正本を確認する**

Run: `git diff --check && rg -n 'password|session_token|password_phc' iotkit-site | less`
Expected: whitespace errorなし。test fixture以外でsecret原文をlog/audit/responseへ出す経路なし。

- [ ] **Step 4: independent reviewへ出す**

認証・認可、session、CSRF、CLI recovery、migration、最後のsystem admin保護を重点review対象にする。
