# Console Sensor Rule Preview Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** センサー設定で表示中tabの保存済みルールまたは作成中ルールだけをプレビュー対象にし、受信値とは別にnumeric・boolean・累積・異常検知の現在結果を明示する。

**Architecture:** SSR templateが各rule formへ安定したpreview IDを与え、tabとdisclosureの状態をfrontend内の単一selectionとして扱う。preview APIは変更せず全候補を一度に評価し、frontendがactive IDと完全一致するresponseだけをsemantic結果として描画する。raw入力は独立した基礎表示として維持し、非表示tabやerrorになった別ruleへfallbackしない。

**Tech Stack:** Rust 2024、Askama SSR、TypeScript、Vitest + jsdom、Playwright、既存IoTKit Edge mapping preview API。

## Global Constraints

- 承認済み設計は`docs/superpowers/specs/2026-07-28-console-sensor-rule-preview-selection-design.md`。判断が衝突したら実装を止め、設計を勝手に拡張しない。
- public OpenAPI、semantic evaluator、storage schema、MQTT、output custody、server-side mutation dispatcherを変更しない。
- preview target IDは保存済みruleの`rule_id`、計測draftの`draft-normal`、異常検知draftの`draft-alarm`に固定する。配列indexから生成しない。
- active targetは表示中の`[data-setting-panel]`内で開いている`details[data-preview-target]`だけから決める。非表示panelを探索しない。
- target未選択、selected target不在、selected target errorのいずれでも別ruleのsemantic結果へfallbackしない。
- raw値、最終受信時刻、通信状態はrule結果から独立して維持する。
- rule結果は色だけに依存せず、`ON` / `OFF`、`累積 N`、`正常` / `異常`を文章で表示する。
- frontend unit testは`edge/frontend/tests/unit/`、Rust integration testは`edge/tests/`に置き、product `src/`へtest bodyを追加しない。
- 各taskでは記載したfocused testを先に失敗させ、最小実装で通してからcommitする。push、PR作成、mergeはこのplanの実行権限に含めない。

---

## Task 1: SSRへpreview targetと異常検知専用作成formを追加する

**Files:**

- Modify: `edge/tests/console_contract.rs`
- Modify: `edge/src/web/templates/console.html`
- Modify: `edge/frontend/static/edge.css`

- [ ] **Step 1: template contractの失敗testを追加する**

`edge/tests/console_contract.rs`の既存sensor detail HTML test群に、tab単位の作成入口とpreview hookを固定するtestを追加する。

```rust
#[tokio::test]
async fn sensor_rule_creation_and_preview_targets_are_scoped_by_tab() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/devices/device-01/sensors/signal-01")
                .header(
                    "cookie",
                    "iotkit_edge_session=valid; iotkit_edge_csrf=csrf",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(response.into_body(), 2_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    assert!(html.contains("data-preview-rule-result"));
    assert!(html.contains("data-preview-rule-name"));
    assert!(html.contains("data-preview-rule-kind"));
    assert!(html.contains("data-preview-rule-value"));
    assert!(html.contains("data-preview-rule-detail"));

    let normal_start = html.find(r#"id="setting-panel-normal""#).unwrap();
    let alarm_start = html.find(r#"id="setting-panel-alarm""#).unwrap();
    let normal = &html[normal_start..alarm_start];
    let alarm = &html[alarm_start..];

    assert!(normal.contains(r#"data-preview-id="draft-normal""#));
    assert!(!normal.contains(r#"<option value="alarm">"#));
    assert!(alarm.contains(r#"id="alarm-rule-create""#));
    assert!(alarm.contains(r#"data-preview-id="draft-alarm""#));
    assert!(alarm.contains(r#"name="kind" value="alarm""#));
    for label in [
        "異常とみなす側",
        "異常になるしきい値",
        "正常に戻るしきい値",
        "異常確定待ち",
        "復帰確定待ち",
    ] {
        assert!(alarm.contains(label), "missing alarm label: {label}");
    }
}
```

- [ ] **Step 2: contract testが新しいhook不足で失敗することを確認する**

Run:

```bash
cargo test -p iotkit-edge --test console_contract sensor_rule_creation_and_preview_targets_are_scoped_by_tab
```

Expected: `data-preview-rule-result`または`alarm-rule-create`が存在しないためFAIL。

- [ ] **Step 3: 受信値と選択rule結果を分けるmarkupを追加する**

`実信号プレビュー`内の現在値block直後へ次の結果領域を追加し、既存graphの凡例へ`data-preview-result-legend`を付ける。

```html
<section class="preview-rule-result" data-preview-rule-result aria-labelledby="preview-rule-result-title">
  <div>
    <p class="section-kicker">SELECTED RULE</p>
    <h3 id="preview-rule-result-title">選択中ルールの結果</h3>
  </div>
  <dl>
    <div><dt>ルール</dt><dd data-preview-rule-name>選択中のルールはありません</dd></div>
    <div><dt>種類</dt><dd data-preview-rule-kind>—</dd></div>
  </dl>
  <strong data-preview-rule-value>—</strong>
  <p data-preview-rule-detail>ルールを開くと判定結果を確認できます。</p>
</section>
```

保存済みcardとformへ同じIDを明示する。

```html
<details class="semantic-rule-card"
         id="rule-{{ rule.rule_id }}"
         data-rule-id="{{ rule.rule_id }}"
         data-preview-target>
  <form class="semantic-form compact-form"
        data-signal-ref="{{ signal.signal_ref }}"
        data-rule-id="{{ rule.rule_id }}"
        data-preview-id="{{ rule.rule_id }}">
```

- [ ] **Step 4: 作成formをtabごとに分離する**

normal tabの作成disclosureをpreview targetにし、`alarm` optionを削除する。

```html
<details id="rule-create" class="semantic-rule-create" data-preview-target>
  <summary>値の使い方を追加</summary>
  <form method="post"
        action="/console/signals/{{ signal.signal_ref }}/semantic-rules"
        class="semantic-form compact-form"
        data-signal-ref="{{ signal.signal_ref }}"
        data-preview-id="draft-normal"
        data-boolean-input="{{ signal.input_is_boolean }}">
    <select name="kind" data-semantic-kind>
      <option value="numeric">測定値</option>
      <option value="boolean">ON / OFF</option>
      <option value="cumulative_counter">累積値</option>
    </select>
  </form>
</details>
```

alarm tabの空状態文を同tab内の作成操作へ合わせ、既存保存済みalarm formと同じfield名で専用作成formを追加する。

```html
<details id="alarm-rule-create" class="semantic-rule-create" data-preview-target>
  <summary>異常検知を追加</summary>
  <form method="post"
        action="/console/signals/{{ signal.signal_ref }}/semantic-rules"
        class="semantic-form compact-form"
        data-signal-ref="{{ signal.signal_ref }}"
        data-preview-id="draft-alarm"
        data-boolean-input="{{ signal.input_is_boolean }}">
    <input type="hidden" name="_csrf" value="{{ csrf }}">
    <input type="hidden" name="kind" value="alarm" data-semantic-kind>
    <label><span>ルール名</span><input name="display_name" required></label>
    <div class="conditional-fields detector-fields" data-semantic-detector>
      <label><span>異常とみなす側</span><select name="detector_mode">
        <option value="boolean_high_active" data-detector-boolean>High（1）を異常にする</option>
        <option value="boolean_low_active" data-detector-boolean>Low（0）を異常にする</option>
        <option value="high_active" data-detector-analog>値が上がった側を異常にする</option>
        <option value="low_active" data-detector-analog>値が下がった側を異常にする</option>
      </select></label>
      <div class="form-row" data-semantic-thresholds>
        <label><span>異常になるしきい値</span><input name="rise_threshold" type="number" step="any" value="0"></label>
        <label><span>正常に戻るしきい値</span><input name="fall_threshold" type="number" step="any" value="0"></label>
      </div>
      <details class="semantic-advanced-settings"><summary>判定の安定化</summary><div class="form-row">
        <label><span>異常確定待ち（秒）</span><input name="rise_debounce_seconds" type="number" step="0.1" min="0" max="300" value="0"></label>
        <label><span>復帰確定待ち（秒）</span><input name="fall_debounce_seconds" type="number" step="0.1" min="0" max="300" value="0"></label>
      </div></details>
    </div>
    <div class="conditional-fields" data-semantic-trigger hidden>
      <select name="trigger"><option value=""></option></select>
    </div>
    <button type="submit">異常検知を追加</button>
  </form>
</details>
```

- [ ] **Step 5: 結果領域のlayoutを既存responsive gridへ統合する**

`edge/frontend/static/edge.css`で`.preview-rule-result`を現在値cardと同じvisual languageに揃え、狭い幅では内容が横にはみ出さないようにする。alarm active用のclassは文字表示を補助する色としてのみ使う。

```css
.preview-rule-result {
  display: grid;
  gap: 0.7rem;
  padding: 1rem;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--surface-subtle);
}

.preview-rule-result > strong {
  font-size: clamp(1.5rem, 4vw, 2.25rem);
  overflow-wrap: anywhere;
}

.preview-rule-result.is-alarm > strong {
  color: var(--danger);
}
```

既存CSS変数名を確認し、存在しない変数は同sectionで既に使われている変数へ置換する。

- [ ] **Step 6: contract testを通してcommitする**

Run:

```bash
cargo test -p iotkit-edge --test console_contract sensor_rule_creation_and_preview_targets_are_scoped_by_tab
git diff --check
```

Expected: focused test PASS、whitespace errorなし。

```bash
git add edge/tests/console_contract.rs edge/src/web/templates/console.html edge/frontend/static/edge.css
git commit -m "feat(console): add scoped sensor rule preview targets"
```

---

## Task 2: tab変更eventとpanel内disclosure排他を実装する

**Files:**

- Modify: `edge/frontend/tests/unit/shell.test.ts`
- Modify: `edge/frontend/src/shell.ts`
- Modify: `edge/frontend/tests/unit/semantic.test.ts`
- Modify: `edge/frontend/src/semantic.ts`

- [ ] **Step 1: tab eventの失敗testを追加する**

`shell.test.ts`のimportへevent名を加える。

```ts
import {
  initializeShell,
  SETTING_TAB_CHANGE_EVENT,
} from "../../src/shell";
```

既存tab fixtureを使い、click後のdetailと表示panelを同時に確認する。

```ts
it("announces the active settings panel after a tab change", () => {
  installSettingTabs();
  const root = document.querySelector<HTMLElement>("[data-setting-tabs]")!;
  const changes: string[] = [];
  root.addEventListener(SETTING_TAB_CHANGE_EVENT, (event) => {
    changes.push((event as CustomEvent<{ key: string }>).detail.key);
  });

  initializeShell();
  document.querySelector<HTMLButtonElement>('[data-setting-tab="alarm"]')!.click();

  expect(changes.at(-1)).toBe("alarm");
  expect(
    document.querySelector<HTMLElement>('[data-setting-panel="alarm"]')!.hidden,
  ).toBe(false);
});
```

- [ ] **Step 2: semantic disclosureの失敗testを追加する**

normalとalarmの二panelを持つfixtureで、各panelの保存済みcardが独立してopenを保持し、同一panelのcreate disclosureを開いた時だけ同panelの保存済みcardが閉じることを固定する。

```ts
it("keeps one preview target open per settings panel", () => {
  document.body.innerHTML = `
    <section data-setting-panel="normal">
      <details class="semantic-rule-card" data-preview-target open></details>
      <details class="semantic-rule-create" data-preview-target></details>
    </section>
    <section data-setting-panel="alarm" hidden>
      <details class="semantic-rule-card" data-preview-target open></details>
      <details class="semantic-rule-create" data-preview-target></details>
    </section>`;

  initializeSemanticForms();
  const normalTargets = document.querySelectorAll<HTMLDetailsElement>(
    '[data-setting-panel="normal"] details[data-preview-target]',
  );
  const alarmCard = document.querySelector<HTMLDetailsElement>(
    '[data-setting-panel="alarm"] .semantic-rule-card',
  )!;
  normalTargets[1].open = true;
  normalTargets[1].dispatchEvent(new Event("toggle"));

  expect(normalTargets[0].open).toBe(false);
  expect(normalTargets[1].open).toBe(true);
  expect(alarmCard.open).toBe(true);
});
```

- [ ] **Step 3: frontend unit testがeventとpanel scope不足で失敗することを確認する**

Run:

```bash
npm exec --prefix edge/frontend -- vitest run --environment happy-dom tests/unit/shell.test.ts tests/unit/semantic.test.ts
```

Expected: exportされていないevent、またはglobalなcard排他によりFAIL。

- [ ] **Step 4: tab activation eventを実装する**

`shell.ts`へ定数をexportし、`activate(key)`がARIA、panel、query stringを更新した後に通知する。

```ts
export const SETTING_TAB_CHANGE_EVENT = "iotkit:setting-tab-change";

root.dispatchEvent(
  new CustomEvent<{ key: string }>(SETTING_TAB_CHANGE_EVENT, {
    detail: { key },
  }),
);
```

初期activateでもeventをdispatchしてよいが、既存URL、keyboard、focus testを壊さない。preview初期化はDOMから初期状態を読むため、初期eventへの依存は作らない。

- [ ] **Step 5: preview target排他をpanel単位に変更する**

`semantic.ts`のglobalな`.semantic-rule-card`処理を、各`[data-setting-panel]`内の`details[data-preview-target]`処理へ置き換える。

```ts
for (const panel of queryAll<HTMLElement>("[data-setting-panel]")) {
  const targets = queryAll<HTMLDetailsElement>(
    "details[data-preview-target]",
    panel,
  );
  const savedCards = targets.filter((target) =>
    target.classList.contains("semantic-rule-card"),
  );
  if (savedCards.length && !targets.some((target) => target.open)) {
    savedCards[0].open = true;
  }
  for (const target of targets) {
    target.addEventListener("toggle", () => {
      if (!target.open) return;
      for (const peer of targets) {
        if (peer !== target) peer.open = false;
      }
    });
  }
}
```

既存kind切替、boolean detector、confirm処理はこのloopの外で維持する。

- [ ] **Step 6: unit testを通してcommitする**

Run:

```bash
npm exec --prefix edge/frontend -- vitest run --environment happy-dom tests/unit/shell.test.ts tests/unit/semantic.test.ts
```

Expected: 対象2 file PASS。

```bash
git add edge/frontend/src/shell.ts edge/frontend/src/semantic.ts edge/frontend/tests/unit/shell.test.ts edge/frontend/tests/unit/semantic.test.ts
git commit -m "fix(console): scope rule selection to active settings tab"
```

---

## Task 3: active target IDとresponse選択を完全一致にする

**Files:**

- Modify: `edge/frontend/tests/unit/preview.test.ts`
- Modify: `edge/frontend/src/preview.ts`

- [ ] **Step 1: active panel、draft ID、no-fallbackの失敗testを追加する**

`installPreviewDOM()`をnormal/alarm panelと`data-preview-target`、`data-preview-id`を持つfixtureへ更新し、次を追加する。

```ts
it("previews only the open target in the visible settings panel", async () => {
  installPreviewDOM();
  const normal = document.querySelector<HTMLElement>(
    '[data-setting-panel="normal"]',
  )!;
  const alarm = document.querySelector<HTMLElement>(
    '[data-setting-panel="alarm"]',
  )!;
  normal.hidden = true;
  alarm.hidden = false;

  initializePreviews();
  await flushPreview();

  expect(createMappingPreviewMock).toHaveBeenLastCalledWith(
    expect.objectContaining({
      rules: expect.arrayContaining([
        expect.objectContaining({ rule_id: "draft-alarm" }),
      ]),
    }),
    expect.any(String),
    expect.any(AbortSignal),
  );
  expect(document.querySelector("[data-preview-rule-name]")?.textContent)
    .toContain("選択中のルールはありません");
});
```

追加testでは、normal draftとalarm draftのrequest IDが固定値であること、selected targetがerrorでも成功した別ruleのkind/valueを表示しないこともassertする。既存mock helper名へ合わせ、timerとfetch flushは既存testの方法を再利用する。

- [ ] **Step 2: preview unit testがhidden rule fallbackで失敗することを確認する**

Run:

```bash
npm exec --prefix edge/frontend -- vitest run --environment happy-dom tests/unit/preview.test.ts
```

Expected: 現行`selectedPreview`が最初の成功ruleへfallbackするためFAIL。

- [ ] **Step 3: selection modelを追加する**

`preview.ts`でactive targetとraw基礎responseを分離する。

```ts
interface PreviewSelection {
  raw: PreviewBody | null;
  selected: SemanticRulePreview | null;
}

function activePreviewID(scope: HTMLElement): string | undefined {
  const activePanel = queryAll<HTMLElement>(
    "[data-setting-panel]",
    scope,
  ).find((panel) => !panel.hidden);
  const form = activePanel?.querySelector<HTMLFormElement>(
    "details[data-preview-target][open] form.semantic-form[data-preview-id]",
  );
  return form?.dataset.previewId;
}

function selectPreview(
  response: MappingPreviewResponse,
  activeID?: string,
): PreviewSelection {
  if (!isMultipleRulePreview(response)) {
    return { raw: response, selected: null };
  }
  const withWindow = (
    rule: SemanticRulePreview | undefined,
  ): SemanticRulePreview | null =>
    rule
      ? {
          ...rule,
          window_start: response.window_start,
          window_end: response.window_end,
          truncated_by: response.truncated_by,
        }
      : null;
  return {
    raw: withWindow(
      response.rules.find((rule) => !rule.error) ?? response.rules[0],
    ),
    selected: withWindow(
      activeID
        ? response.rules.find((rule) => rule.rule_id === activeID)
        : undefined,
    ),
  };
}
```

`raw`をgraphへ渡す前にsemantic overlayを除くhelperを作る。`kind`、threshold、active、counterに由来する表示を残さず、受信系列だけを使う。

```ts
function rawOnlyPreview(payload: PreviewBody): PreviewBody {
  return {
    ...payload,
    kind: "numeric",
    rise_threshold: undefined,
    fall_threshold: undefined,
    points: (payload.points ?? []).map((point) => ({
      ...point,
      calibrated: point.input,
      calibrated_min: point.input_min,
      calibrated_max: point.input_max,
      active: undefined,
      active_samples: undefined,
      transitions: undefined,
      counter: undefined,
      increment: undefined,
    })),
  };
}
```

generated typeがoptional propertyへの`undefined`代入を許さない場合はdestructuringでsemantic fieldを除外し、同じ返却内容にする。

- [ ] **Step 4: requestへ安定IDとactive empty draftを含める**

`buildRequest`へ`activeID`を渡し、formの`data-preview-id`を唯一のID sourceにする。

```ts
const rules = forms
  .filter((candidate) => {
    const previewID = candidate.dataset.previewId;
    const hasName = !!formField(candidate, "display_name")?.value.trim();
    return !!previewID && (hasName || previewID === activeID);
  })
  .map((candidate) => ({
    rule_id: candidate.dataset.previewId!,
    display_name:
      formField(candidate, "display_name")?.value.trim() ||
      (candidate.dataset.previewId === "draft-alarm"
        ? "新しい異常検知"
        : "新しい計測ルール"),
    spec: ruleSpec(candidate),
  }));
```

raw基礎dataを取得するため、候補が0件の時だけ内部numeric ruleを加える。

```ts
if (!rules.length) {
  rules.push({
    rule_id: "draft-raw",
    display_name: "受信値",
    spec: { kind: "numeric" },
  });
}
```

実際の`SemanticRuleSpec`必須fieldはgenerated typeと`ruleSpec`を確認し、typecheckが通る最小numeric specを指定する。この内部IDをactive targetとして扱わない。

- [ ] **Step 5: tabとdisclosureの変更でrefreshする**

`preview.ts`から`SETTING_TAB_CHANGE_EVENT`をimportし、preview scopeへevent listenerを追加する。各`details[data-preview-target]`の`toggle`でも既存debounce経由でrefreshする。

```ts
previewScope.addEventListener(SETTING_TAB_CHANGE_EVENT, scheduleRefresh);
for (const target of queryAll<HTMLDetailsElement>(
  "details[data-preview-target]",
  previewScope,
)) {
  target.addEventListener("toggle", scheduleRefresh);
}
```

`refresh()`冒頭で一度だけ`const activeID = activePreviewID(previewScope)`を求め、requestとresponse selectionの両方へ同じ値を渡す。selected target不在・error時にはraw-only graphを描画し、別ruleをselectedへ代入しない。

validation errorのfield探索も`forms[0]`ではなくactive IDと一致するformへ限定する。

```ts
const activeForm = forms.find(
  (candidate) => candidate.dataset.previewId === activeID,
);
const invalidField =
  fieldName && activeForm ? formField(activeForm, fieldName) : null;
```

成功responseの描画対象は次の条件で決める。raw現在値は常に`selection.raw`の最新`input`から更新する。

```ts
const selection = selectPreview(result.value, activeID);
const selectedReady = selection.selected && !selection.selected.error
  ? selection.selected
  : null;
const chartPayload = selectedReady
  ?? (selection.raw ? rawOnlyPreview(selection.raw) : null);
const resultState = !activeID
  ? "none"
  : selectedReady
    ? "ready"
    : "error";
```

- [ ] **Step 6: selection testとtypecheckを通してcommitする**

Run:

```bash
npm exec --prefix edge/frontend -- vitest run --environment happy-dom tests/unit/preview.test.ts
npm exec --prefix edge/frontend -- tsc --noEmit
```

Expected: preview unit testとTypeScript compile PASS。

```bash
git add edge/frontend/src/preview.ts edge/frontend/tests/unit/preview.test.ts
git commit -m "fix(console): select preview response by active rule"
```

---

## Task 4: kind別の現在結果と読み上げsummaryを描画する

**Files:**

- Modify: `edge/frontend/tests/unit/preview.test.ts`
- Modify: `edge/frontend/src/preview.ts`
- Modify: `edge/frontend/static/edge.css`

- [ ] **Step 1: 4 kindと空・error状態の失敗testを追加する**

response fixtureへ`active`、`counter`、`increment`を指定できるbuilderを用意し、次の表示を固定する。

```ts
it.each([
  ["numeric", { calibrated: 24.5 }, "24.5 ℃"],
  ["boolean", { active: true }, "ON"],
  ["cumulative_counter", { counter: 42, increment: 1 }, "累積 42"],
  ["alarm", { active: false }, "正常"],
  ["alarm", { active: true }, "異常"],
])("renders the selected %s outcome", async (kind, point, expected) => {
  installPreviewDOM();
  previewResponseMock.mockResolvedValue(
    okPreviewResponse({ kind, point, displayName: "確認対象" }),
  );

  initializePreviews();
  await flushPreview();

  expect(document.querySelector("[data-preview-rule-value]")?.textContent)
    .toContain(expected);
  expect(
    document.querySelector("[data-preview-accessible-summary]")?.textContent,
  ).toContain("確認対象");
  expect(
    document.querySelector("[data-preview-accessible-summary]")?.textContent,
  ).toContain(expected);
});
```

別testでtarget未選択=`選択中のルールはありません`、selected error=`判定結果を更新できません`、空points=`受信待ち`、validation error=`設定内容を確認してください`をassertする。request全体が失敗した時は`data-preview-current-value`が直前値を保つことも固定する。

- [ ] **Step 2: 結果表示不足でtestが失敗することを確認する**

Run:

```bash
npm exec --prefix edge/frontend -- vitest run --environment happy-dom tests/unit/preview.test.ts
```

Expected: `data-preview-rule-value`が更新されずFAIL。

- [ ] **Step 3: kind別outcome formatterを実装する**

`SemanticKind`を`semantic.ts`からtype importし、最新pointを文章へ変換する。

```ts
interface RuleOutcome {
  value: string;
  detail: string;
  alarm: boolean;
}

function latestRuleOutcome(
  payload: PreviewBody,
  unit: string,
): RuleOutcome {
  const latest = payload.points?.at(-1);
  if (!latest) {
    return { value: "受信待ち", detail: "受信データを待っています。", alarm: false };
  }
  switch (payload.kind) {
    case "boolean":
      return {
        value: latest.active ? "ON" : "OFF",
        detail: "現在の判定",
        alarm: false,
      };
    case "cumulative_counter":
      return {
        value: `累積 ${formatNumber(latest.counter ?? 0)}`,
        detail: Number(latest.increment ?? 0) > 0
          ? `今回 +${formatNumber(latest.increment)}`
          : "今回の増分なし",
        alarm: false,
      };
    case "alarm":
      return {
        value: latest.active ? "異常" : "正常",
        detail: latest.active ? "異常条件に該当" : "正常範囲",
        alarm: Boolean(latest.active),
      };
    default:
      return {
        value: `${formatNumber(latest.calibrated)}${unit ? ` ${unit}` : ""}`,
        detail: "補正後の値",
        alarm: false,
      };
  }
}
```

- [ ] **Step 4: rule結果領域と試す値の結果を更新する**

`renderRuleResult`を追加し、rule名、kind label、value、detailを一か所で更新する。kind labelは`測定値`、`ON / OFF`、`累積値`、`異常検知`の固定mapを使う。

```ts
const kindLabels: Record<SemanticKind, string> = {
  numeric: "測定値",
  boolean: "ON / OFF",
  cumulative_counter: "累積値",
  alarm: "異常検知",
};

function kindLabel(kind: SemanticKind): string {
  return kindLabels[kind];
}

function renderRuleResult(
  panel: HTMLElement,
  selected: SemanticRulePreview | null,
  state: "ready" | "none" | "invalid" | "error",
  unit: string,
): RuleOutcome | null {
  const container = query<HTMLElement>("[data-preview-rule-result]", panel);
  const name = query<HTMLElement>("[data-preview-rule-name]", panel);
  const kind = query<HTMLElement>("[data-preview-rule-kind]", panel);
  const value = query<HTMLElement>("[data-preview-rule-value]", panel);
  const detail = query<HTMLElement>("[data-preview-rule-detail]", panel);
  if (!container || !name || !kind || !value || !detail) return null;

  container.classList.remove("is-alarm");
  if (state !== "ready" || !selected) {
    const messages = {
      none: ["選択中のルールはありません", "—", "ルールを開くと判定結果を確認できます。"],
      invalid: ["設定内容を確認してください", "—", "入力項目を修正してください。"],
      error: ["判定結果を更新できません", "—", "受信値はそのまま確認できます。"],
    } as const;
    const [title, result, hint] = messages[state === "ready" ? "none" : state];
    setText(name, title);
    setText(kind, "—");
    setText(value, result);
    setText(detail, hint);
    return null;
  }

  const outcome = latestRuleOutcome(selected, unit);
  setText(name, selected.display_name);
  setText(kind, kindLabel(selected.kind));
  setText(value, outcome.value);
  setText(detail, outcome.detail);
  container.classList.toggle("is-alarm", outcome.alarm);
  return outcome;
}
```

test inputの判定も同じsemantic vocabularyへ揃える。alarmなら`正常` / `異常`、booleanなら`ON` / `OFF`、累積ならpreview responseの`counter`と`increment`を使う。

- [ ] **Step 5: accessibility summaryへ選択結果を含める**

`updateAccessibleSummary`の引数をraw payload、selected rule、outcomeへ分ける。

```ts
function updateAccessibleSummary(
  summary: HTMLElement | null,
  raw: PreviewBody,
  selected: SemanticRulePreview | null,
  outcome: RuleOutcome | null,
): void {
  if (!summary) return;
  const points = raw.points ?? [];
  if (!points.length) {
    setText(
      summary,
      selected
        ? `${selected.display_name}は受信データを待っています。`
        : "グラフに表示できる受信データはまだありません。",
    );
    return;
  }
  const inputs = points.flatMap((point) => [
    Number(point.input_min),
    Number(point.input_max),
  ]);
  const count = raw.input_count ?? points.length;
  const ruleText = selected && outcome
    ? `選択中は${selected.display_name}、${kindLabel(selected.kind)}、現在は${outcome.value}です。`
    : "選択中のルールはありません。";
  setText(
    summary,
    `受信値は${formatNumber(Math.min(...inputs))}から` +
      `${formatNumber(Math.max(...inputs))}です。${ruleText}` +
      `${count}件の受信データを表示しています。`,
  );
}
```

- [ ] **Step 6: resultとaccessibility testを通してcommitする**

Run:

```bash
npm exec --prefix edge/frontend -- vitest run --environment happy-dom tests/unit/preview.test.ts
npm run check --prefix edge/frontend
```

Expected: preview testを含むfrontend check PASS。

```bash
git add edge/frontend/src/preview.ts edge/frontend/tests/unit/preview.test.ts edge/frontend/static/edge.css
git commit -m "feat(console): show selected sensor rule outcome"
```

---

## Task 5: 実Console journeyでtab、draft、保存後selectionを固定する

**Files:**

- Modify: `edge/frontend/e2e/rust-console-journey.mjs`

- [ ] **Step 1: 照度の空alarm tabを再現する失敗journeyを追加する**

既存patrol lamp fixtureで累積ruleを削除する前にalarm tabを開き、別tabの累積結果が出ないことと専用作成入口を確認する。

```js
await page.getByRole("tab", { name: /異常検知/ }).click();
await expect(page.locator("[data-preview-rule-name]"))
  .toHaveText("選択中のルールはありません");
await expect(page.locator("[data-preview-rule-value]"))
  .not.toContainText("累積");
await expect(page.getByText("異常検知を追加", { exact: true }))
  .toBeVisible();
```

- [ ] **Step 2: alarm draft作成、preview、保存、reload journeyを追加する**

alarm disclosureを開き、ルール名とthresholdを入力して結果を確認し、保存後もalarm tabの保存済みcardとpreview名が一致することをassertする。

```js
await page.getByText("異常検知を追加", { exact: true }).click();
const alarmDraft = page.locator("#alarm-rule-create");
await alarmDraft.getByLabel("ルール名").fill("照度異常");
await alarmDraft.getByLabel("異常になるしきい値").fill("900");
await alarmDraft.getByLabel("正常に戻るしきい値").fill("850");
await expect(page.locator("[data-preview-rule-name]")).toHaveText("照度異常");
await alarmDraft.getByRole("button", { name: "異常検知を追加" }).click();
await page.waitForLoadState("networkidle");
await expect(page.getByRole("tab", { name: /異常検知/ }))
  .toHaveAttribute("aria-selected", "true");
await expect(page.locator("[data-preview-rule-name]")).toHaveText("照度異常");
```

保存後のquery parameterが現行routeでalarm tabを復元することもassertする。fixture cleanupは既存retire flowを使い、journeyの後続testへruleを残さない。

- [ ] **Step 3: 温度alarmと接点boolean/cumulativeの表示journeyを追加する**

温度の保存済み高温alarmを開き、`正常`または`異常`の文字が表示されることを確認する。接点入力ではboolean cardとcumulative cardを順に開き、結果が`ON` / `OFF`から`累積 N`へ変わることを確認する。

```js
await expect(page.locator("[data-preview-rule-value]"))
  .toHaveText(/^(正常|異常)$/);

await booleanCard.locator("summary").click();
await expect(page.locator("[data-preview-rule-value]"))
  .toHaveText(/^(ON|OFF)$/);
await cumulativeCard.locator("summary").click();
await expect(page.locator("[data-preview-rule-value]"))
  .toHaveText(/^累積 /);
```

card locatorはrule名を含む`details.semantic-rule-card`から取得し、DOM順へ依存しない。

- [ ] **Step 4: E2Eが現行fallbackまたはmissing formで失敗することを確認する**

Run:

```bash
scripts/test-edge-console-e2e.sh
```

Expected: 実装前のcheckpointでは空alarm tabまたは`#alarm-rule-create`でFAIL。Task 1–4実装後はPASSする。

- [ ] **Step 5: browser exceptionとaccessibility summaryも固定する**

既存page error collectorを維持し、各kind切替後にsummaryがrule名と表示結果を含むことをassertする。

```js
await expect(page.locator("[data-preview-accessible-summary]"))
  .toContainText("照度異常");
await expect(page.locator("[data-preview-accessible-summary]"))
  .toContainText(/正常|異常/);
```

- [ ] **Step 6: E2Eを通してcommitする**

Run:

```bash
scripts/test-edge-console-e2e.sh
git diff --check
```

Expected: production Rust server + fixtureのjourney PASS、browser exception 0、whitespace errorなし。

```bash
git add edge/frontend/e2e/rust-console-journey.mjs
git commit -m "test(console): cover sensor rule preview selection journey"
```

---

## Task 6: 回帰検証、battle-tested review、全体verificationを完了する

**Files:**

- Verify only unless a failing check identifies an in-scope defect.

- [ ] **Step 1: focused frontendとRust contractを再実行する**

Run:

```bash
npm run check --prefix edge/frontend
cargo test -p iotkit-edge --test console_contract
```

Expected: frontend unit/type/style checks PASS、`console_contract`全test PASS。

- [ ] **Step 2: repository wrapperとsource placementを検証する**

Run:

```bash
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh
scripts/check-source-layout
```

Expected: frontend wrapper、browser journey、source/test placementがすべてPASS。

- [ ] **Step 3: battle-tested selectorを実行して選択項目をreviewする**

Run:

```bash
node scripts/battle-tested-review.mjs select --base origin/master
```

Expected: 出力された`BT-NNN`だけを`review/battle-tested/`の記載に従って確認する。選択0件でも、hidden tab fallback、selected error、raw保持、browser exceptionのsemantic reviewは継続する。

- [ ] **Step 4: Rust product behaviorを含む全体verificationを実行する**

Run:

```bash
scripts/verify.sh
```

Expected: rustfmt、layer rules、workspace tests、Clippy `-D warnings`がPASS。Windows nativeでは既存Unix permission APIが成立しないため、WSLまたはLinux devcontainerで実行する。

- [ ] **Step 5: 承認済み設計とdiffを照合する**

Run:

```bash
git diff origin/master...HEAD --stat
git diff origin/master...HEAD -- edge/src/web/templates/console.html edge/frontend/src edge/frontend/tests edge/tests/console_contract.rs
git diff --check
git status --short
```

次を目視確認する。

- normal tabにalarm作成候補がない。
- alarm tab内だけでalarmを作成できる。
- active target IDが保存済みID、`draft-normal`、`draft-alarm`のいずれかである。
- hidden panel、missing response、selected errorから別ruleへfallbackしない。
- raw現在値と選択rule結果が独立している。
- numeric、boolean、cumulative、alarmの文章表示と読み上げsummaryがある。
- public API、storage、MQTT、mutation routeへ差分がない。

- [ ] **Step 6: verificationで必要になった小修正だけをcommitする**

check修正が発生した場合だけ実行する。

```bash
git add edge/src/web/templates/console.html edge/frontend edge/tests/console_contract.rs
git commit -m "fix(console): address sensor preview verification"
```

Expected: `git status --short`が空。修正がなければcommitは作らない。
