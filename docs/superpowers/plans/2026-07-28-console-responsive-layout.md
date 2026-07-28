# IoTKit Console Responsive Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make all principal IoTKit Console pages usable without document-level horizontal overflow or an inoperable navigation drawer from 360px through narrow desktop widths.

**Architecture:** Keep the existing SSR templates, CSS, and TypeScript shell. Repair the shared drawer contract first, add a failing real-browser responsive audit, then make the smallest template and CSS changes that satisfy it across the eight principal pages.

**Tech Stack:** Rust/Axum/Askama SSR, TypeScript, Vitest with happy-dom, CSS media queries, Chromium DevTools Protocol E2E.

## Global Constraints

- Compact mode is `max-width: 960px`; desktop mode begins at 961px.
- Verify `/status`, `/sensors`, `/logs`, `/equipment`, `/output`, `/audit`, `/accounts`, and `/system` at 390px, 768px, and 1024px.
- Dense log and audit tables may scroll only inside `.table-wrap`; the document must not scroll horizontally.
- Viewport changes must never remove data, actions, authorization checks, CSRF fields, or server-side error content.
- Do not change HTTP routes, request/response formats, sessions, authorization, MQTT, storage, or semantic projection.
- Keep frontend tests under `edge/frontend/tests/`; keep Rust integration tests under `edge/tests/`.
- Implement behavior test-first and commit each independently reviewable task.

---

### Task 1: Restore the accessible mobile navigation drawer

**Files:**
- Modify: `edge/frontend/tests/unit/shell.test.ts`
- Modify: `edge/frontend/src/shell.ts`
- Modify: `edge/src/web/templates/console.html`
- Modify: `edge/tests/console_contract.rs`

**Interfaces:**
- Consumes: `.menu-button`, `#sidebar`, `.side-nav`, and `.mobile-overlay` from the Console template.
- Produces: `initializeShell()` drawer behavior with `body.menu-open`, synchronized `aria-expanded`, focus entry/return, and desktop breakpoint cleanup.

- [ ] **Step 1: Write failing frontend tests**

Add this fixture to `shell.test.ts`:

```ts
function renderMobileShell(): void {
  document.body.innerHTML = `
    <button class="menu-button" aria-controls="sidebar" aria-expanded="false">
      メニュー
    </button>
    <aside class="sidebar" id="sidebar">
      <nav class="side-nav">
        <a class="active" href="/status">概要</a>
        <a href="/equipment">機器管理</a>
      </nav>
    </aside>
    <button class="mobile-overlay" type="button" hidden
      aria-label="メニューを閉じる"></button>
  `;
}
```

Test button open/close, overlay close, Escape close, navigation-link close,
focus entry/return, desktop `MediaQueryList` change cleanup, and missing-node
no-op. Stub `window.matchMedia` with a listener-capturing object.

- [ ] **Step 2: Run frontend tests and confirm RED**

```powershell
npm test --prefix edge/frontend -- --run tests/unit/shell.test.ts
```

Expected: the new focus and breakpoint assertions fail.

- [ ] **Step 3: Write a failing SSR contract**

Extend `console_shell_preserves_accessible_navigation_and_authentication`:

```rust
for hook in [
    r#"class="menu-button""#,
    r#"aria-controls="sidebar""#,
    r#"class="mobile-overlay""#,
    r#"aria-label="メニューを閉じる""#,
] {
    assert!(html.contains(hook), "missing {hook}");
}
```

- [ ] **Step 4: Run the contract and confirm RED**

```powershell
cargo test -p iotkit-edge --test console_contract console_shell_preserves_accessible_navigation_and_authentication
```

Expected: FAIL with `missing class="mobile-overlay"`.

- [ ] **Step 5: Implement the shared drawer**

Add one overlay after the sidebar:

```html
<button class="mobile-overlay" type="button" hidden
  aria-label="メニューを閉じる"></button>
```

Refactor `initializeMenu()` around one setter:

```ts
const compactLayout = window.matchMedia("(max-width: 960px)");
const setOpen = (open: boolean, restoreFocus = false): void => {
  document.body.classList.toggle("menu-open", open);
  menuButton.setAttribute("aria-expanded", String(open));
  overlay.hidden = !open;
  if (open) {
    (
      query<HTMLAnchorElement>(".side-nav a.active", sidebar) ??
      query<HTMLAnchorElement>(".side-nav a", sidebar)
    )?.focus();
  } else if (restoreFocus) {
    menuButton.focus();
  }
};
```

Bind button, overlay, Escape, `.side-nav a`, and media-query `change` events.
When `matches` becomes false, close without moving focus. Preserve no-op when a
required node is absent.

- [ ] **Step 6: Run focused tests and confirm GREEN**

```powershell
npm test --prefix edge/frontend -- --run tests/unit/shell.test.ts
cargo test -p iotkit-edge --test console_contract console_shell_preserves_accessible_navigation_and_authentication
```

Expected: both commands pass.

- [ ] **Step 7: Commit**

```powershell
git add edge/frontend/tests/unit/shell.test.ts edge/frontend/src/shell.ts edge/src/web/templates/console.html edge/tests/console_contract.rs
git commit -m "fix(console): restore responsive navigation drawer"
```

---

### Task 2: Add a failing responsive browser audit

**Files:**
- Create: `edge/frontend/e2e/responsive-console.mjs`
- Modify: `edge/frontend/e2e/rust-console-journey.mjs`
- Modify: `edge/frontend/e2e/console-journey.mjs`

**Interfaces:**
- Consumes: a DevTools-like object with `send(method, params)` and
  `evaluate(expression)`, plus a `navigate(path)` callback.
- Produces: `verifyResponsiveConsole({ devtools, navigate })`, which throws a
  path/width-specific error on overflow, clipping, uncontained dense tables, or
  broken drawer behavior.

- [ ] **Step 1: Create the reusable audit**

Create `responsive-console.mjs`:

```js
export const responsiveConsolePaths = [
  "/status",
  "/sensors",
  "/logs",
  "/equipment",
  "/output",
  "/audit",
  "/accounts",
  "/system",
];

export async function verifyResponsiveConsole({ devtools, navigate }) {
  try {
    for (const width of [390, 768, 1024]) {
      await devtools.send("Emulation.setDeviceMetricsOverride", {
        width,
        height: 844,
        deviceScaleFactor: 1,
        mobile: width <= 768,
      });
      for (const path of responsiveConsolePaths) {
        await navigate(path);
        const state = await devtools.evaluate(responsiveStateExpression);
        if (!state.documentFits || state.clipped.length || !state.tablesContained) {
          throw new Error(
            `${path} is not responsive at ${width}px: ${JSON.stringify(state)}`,
          );
        }
      }
    }
    await verifyDrawer({ devtools, navigate });
  } finally {
    await devtools.send("Emulation.clearDeviceMetricsOverride");
  }
}
```

The geometry expression must ignore descendants of `.table-wrap`, but inspect
visible principal cards, toolbars, equipment rows, status rows, account rows,
buttons, and form controls. It must require `#log-table` and `#audit-table` to
live inside a computed `overflow-x: auto` wrapper.

The drawer audit must exercise button open, Escape close, overlay close,
navigation close, focus entry/return, and cleanup when switching from 390px to
1024px.

- [ ] **Step 2: Invoke it from both owner journeys**

Import:

```js
import { verifyResponsiveConsole } from "./responsive-console.mjs";
```

In `rust-console-journey.mjs`:

```js
await verifyResponsiveConsole({
  devtools,
  navigate: (path) => devtools.navigate(path),
});
```

In `console-journey.mjs`:

```js
await verifyResponsiveConsole({
  devtools,
  navigate: (path) => devtools.navigate(`${edgeNodeURL}${path}`, path),
});
```

Keep output-specific long topic/payload assertions, but remove the superseded
output-only document-width checks.

- [ ] **Step 3: Run E2E and confirm RED**

```powershell
scripts/test-edge-console-e2e.sh
```

Expected: FAIL on the first known responsive defect, such as missing
`.table-wrap`, `/status` overflow, or `/accounts` overflow. Preserve the exact
failure as RED evidence before Task 3.

---

### Task 3: Make principal Console pages compact-safe

**Files:**
- Modify: `edge/src/web/templates/console.html`
- Modify: `edge/src/web/templates/signal-table.html`
- Modify: `edge/frontend/static/edge.css`
- Modify: `edge/tests/console_contract.rs`
- Test: `edge/frontend/e2e/responsive-console.mjs`

**Interfaces:**
- Consumes: unchanged `ConsoleView` fields and form actions.
- Produces: `.status-signal-table`, `.signal-table-wrap`, `.table-wrap`,
  `.account-table`, and compact card layouts required by Task 2.

- [ ] **Step 1: Add failing layout-hook contracts**

In `console_pages_render_the_existing_operator_content_and_form_hooks`, require:

```rust
("/status", &[r#"class="signal-table-wrap status-signal-table""#][..]),
("/logs", &[r#"class="table-wrap history-table-wrap""#][..]),
("/audit", &[r#"class="table-wrap audit-table-wrap""#][..]),
("/accounts", &[r#"class="account-table""#, r#"data-label="ログインID""#][..]),
```

Add `/accounts` and `/audit` route cases if absent.

- [ ] **Step 2: Run the contract and confirm RED**

```powershell
cargo test -p iotkit-edge --test console_contract console_pages_render_the_existing_operator_content_and_form_hooks
```

Expected: FAIL because the hooks do not exist.

- [ ] **Step 3: Add semantic wrappers and labels**

Use:

```html
<div class="signal-table-wrap status-signal-table">
  {% include "signal-table.html" %}
</div>
```

Use `.signal-table-wrap` around the `/sensors` include. Wrap `#log-table` and
`#audit-table` in named `.table-wrap` containers. Add
`<table class="account-table">`, a real `<thead>`, and `data-label` on account
cells. Keep form methods, actions, `_csrf`, revisions, and confirmation hooks
unchanged.

Add `data-label` to the four cells in `signal-table.html`:

```html
<td data-label="センサー">...</td>
<td data-label="現在値">...</td>
<td data-label="受信状態">...</td>
<td data-label="収集ノード">...</td>
```

- [ ] **Step 4: Implement compact CSS**

In `edge.css`:

- change `@media (max-width: 780px)` to `@media (max-width: 960px)`;
- keep the existing 460px spacing override;
- add `.signal-table-wrap { width: 100%; min-width: 0; }`;
- set dense history/audit tables to `min-width: 760px`;
- remove the dead `tr[data-href]` requirement from sensor-list compact selectors;
- apply the same four-cell stacked layout to `.status-signal-table`;
- render account rows as labelled cards with stacked forms and full-width controls;
- render `.account-create-form` as a one-column grid in compact mode;
- ensure `.mobile-overlay[hidden]` stays hidden and the drawer is above it.

Do not hide overflow globally on `html` or `body`.

- [ ] **Step 5: Run focused contracts and frontend checks**

```powershell
cargo test -p iotkit-edge --test console_contract console_pages_render_the_existing_operator_content_and_form_hooks
npm run check --prefix edge/frontend
```

Expected: both commands pass.

- [ ] **Step 6: Run E2E and confirm GREEN**

```powershell
scripts/test-edge-console-e2e.sh
```

Expected: Rust preflight and the 390px/768px/1024px eight-page journey pass
without browser exceptions.

- [ ] **Step 7: Commit Task 2 and Task 3 together**

The Task 2 audit was intentionally left uncommitted while RED. Commit it with
the minimal implementation that turns it GREEN:

```powershell
git add edge/frontend/e2e/responsive-console.mjs edge/frontend/e2e/rust-console-journey.mjs edge/frontend/e2e/console-journey.mjs edge/src/web/templates/console.html edge/src/web/templates/signal-table.html edge/frontend/static/edge.css edge/tests/console_contract.rs
git commit -m "fix(console): adapt principal pages to compact widths"
```

---

### Task 4: Run gates, independent review, and publish the draft PR

**Files:**
- Modify only if verification reveals an Issue #105 regression.
- Review: `review/battle-tested/README.md`

**Interfaces:**
- Consumes: all implementation commits.
- Produces: a verified branch and draft PR closing Issue #105.

- [ ] **Step 1: Run focused verification**

```powershell
npm run check --prefix edge/frontend
cargo test -p iotkit-edge --test console_contract --test http_contract
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh
scripts/check-source-layout
```

Expected: all commands exit 0.

- [ ] **Step 2: Run broad Rust verification**

```powershell
scripts/verify.sh
```

Expected: formatting, layer/source rules, workspace tests, and Clippy with
`-D warnings` all exit 0.

- [ ] **Step 3: Run Battle-Tested review routing**

```powershell
node scripts/battle-tested-review.mjs select --base origin/master
node scripts/battle-tested-review.mjs check
```

Review every selected `BT-NNN` entry and record whether field evidence should
update a regression test or runbook.

- [ ] **Step 4: Inspect final scope**

```powershell
git diff --check origin/master...HEAD
git diff --stat origin/master...HEAD
git status --short --branch
```

Expected: only Issue #105 design/plan, Console presentation, focused tests, and
browser journey files changed; the worktree is clean.

- [ ] **Step 5: Request repository-local independent review**

Review correctness, accessibility, responsive geometry, test quality, and
Issue #105 scope. Resolve every Critical or Important finding on the same
branch and rerun affected gates.

- [ ] **Step 6: Push and open a draft PR**

```powershell
git push -u origin agent/issue-105-responsive-console
```

Open a draft PR against `master` with `Closes #105`, verification evidence,
Battle-Tested result, and any skipped check. Do not merge without separate
approval.
