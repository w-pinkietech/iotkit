import { afterEach, describe, expect, it, vi } from "vitest";
import { initializePreviews } from "../../src/preview";
import { SETTING_TAB_CHANGE_EVENT } from "../../src/shell";

function installPreviewDOM(): void {
  document.body.innerHTML = `
    <div class="sensor-detail-latest" data-source-value="0">
      <span data-source-current-value>0</span>
      <span data-source-current-received>未受信</span>
    </div>
    <div class="sensor-setting-workspace">
      <form action="/console/signals/sig_01/calibration">
        <input name="scale" value="1">
        <input name="offset" value="0">
      </form>
      <section data-setting-panel="normal">
        <details class="semantic-rule-card" data-rule-id="rule-01"
          data-preview-target open>
          <form class="semantic-form"
            data-signal-ref="sig_01"
            data-rule-id="rule-01"
            data-preview-id="rule-01"
            action="/console/semantic-rules/rule-01">
            <input name="display_name" value="温度">
            <input name="kind" value="numeric">
            <select name="detector_mode"><option value="" selected>none</option></select>
            <label><span>立ち上がりしきい値</span>
              <input name="rise_threshold" value="0">
            </label>
            <input name="fall_threshold" value="0">
            <input name="rise_debounce_seconds" value="0">
            <input name="fall_debounce_seconds" value="0">
            <select name="trigger"><option value="" selected>none</option></select>
          </form>
        </details>
        <details class="semantic-rule-create" data-preview-target>
          <form class="semantic-form"
            data-signal-ref="sig_01"
            data-preview-id="draft-normal"
            action="/console/signals/sig_01/semantic-rules">
            <input name="display_name" value="">
            <input name="kind" value="numeric">
            <select name="detector_mode"><option value="" selected>none</option></select>
            <input name="rise_threshold" value="0">
            <input name="fall_threshold" value="0">
            <input name="rise_debounce_seconds" value="0">
            <input name="fall_debounce_seconds" value="0">
            <select name="trigger"><option value="" selected>none</option></select>
          </form>
        </details>
      </section>
      <section data-setting-panel="alarm" hidden>
        <details class="semantic-rule-create" data-preview-target open>
          <form class="semantic-form"
            data-signal-ref="sig_01"
            data-preview-id="draft-alarm"
            action="/console/signals/sig_01/semantic-rules">
            <input name="display_name" value="">
            <input name="kind" value="alarm">
            <select name="detector_mode"><option value="high_active" selected>high</option></select>
            <input name="rise_threshold" value="10">
            <input name="fall_threshold" value="5">
            <input name="rise_debounce_seconds" value="0">
            <input name="fall_debounce_seconds" value="0">
            <select name="trigger"><option value="" selected>none</option></select>
          </form>
        </details>
      </section>
      <section data-setting-simulation data-signal-ref="sig_01" data-unit="℃">
        <button type="button" role="switch" aria-checked="true"
          data-preview-toggle>自動更新</button>
        <span data-preview-range></span>
        <span data-preview-count></span>
        <span data-preview-message></span>
        <span data-preview-feed-state>受信データを確認中</span>
        <span data-preview-checked-at></span>
        <p class="sr-only" id="sensor-preview-chart-summary"
          data-preview-accessible-summary>受信データを読み込んでいます。</p>
        <strong data-preview-current-value></strong>
        <span data-preview-current-received></span>
        <svg data-preview-chart aria-describedby="sensor-preview-chart-summary"></svg>
        <span data-preview-result-legend hidden>設定結果</span>
        <span data-preview-threshold-legend hidden>立上り・立下り</span>
        <section class="simulation-counter" data-preview-counter hidden
          aria-labelledby="sensor-preview-counter-title">
          <div class="simulation-summary">
            <strong id="sensor-preview-counter-title"
              data-preview-counter-label>保存済み累積値</strong>
            <span id="sensor-preview-counter-summary"
              data-preview-counter-summary>保存済み履歴を読み込んでいます。</span>
          </div>
          <div class="simulation-chart-wrap">
            <svg data-preview-counter-chart
              aria-describedby="sensor-preview-counter-summary"></svg>
          </div>
        </section>
        <section data-preview-rule-result>
          <label class="preview-rule-select"><span>プレビューするルール</span>
            <select data-preview-rule-select disabled>
              <option value="" selected disabled>選択できるルールなし</option>
            </select>
          </label>
          <dd data-preview-rule-name>選択中のルールはありません</dd>
          <dd data-preview-rule-kind>—</dd>
          <strong data-preview-rule-value>—</strong>
          <p data-preview-rule-detail>ルールを選択すると判定結果を確認できます。</p>
        </section>
      </section>
    </div>
  `;
  document.cookie = "iotkit_edge_csrf=csrf-test";
}

function installNoRulePreviewDOM(): void {
  installPreviewDOM();
  document.querySelector("details.semantic-rule-card")?.remove();
  const form = document.createElement("form");
  form.className = "semantic-form";
  form.dataset.signalRef = "sig_01";
  form.action = "/console/signals/sig_01/semantic-rules";
  form.innerHTML = `
    <input name="display_name" value="">
    <input name="kind" value="numeric">
    <select name="detector_mode"><option value="" selected>none</option></select>
    <input name="rise_threshold" value="0">
    <input name="fall_threshold" value="0">
    <input name="rise_debounce_seconds" value="0">
    <input name="fall_debounce_seconds" value="0">
    <select name="trigger"><option value="" selected>none</option></select>
  `;
  document.body.append(form);
}

function installNoPreviewTargetDOM(): void {
  installPreviewDOM();
  for (const target of document.querySelectorAll("details[data-preview-target]")) {
    target.remove();
  }
}

type PreviewKind = "numeric" | "boolean" | "cumulative_counter" | "alarm";

function setSavedRuleKind(kind: PreviewKind): void {
  document.querySelector<HTMLInputElement>(
    'form.semantic-form[data-rule-id] [name="kind"]',
  )!.value = kind;
}

function addSavedRule(
  ruleID: string,
  displayName: string,
): HTMLDetailsElement {
  const first = document.querySelector<HTMLDetailsElement>(
    "details.semantic-rule-card",
  )!;
  const card = first.cloneNode(true) as HTMLDetailsElement;
  card.dataset.ruleId = ruleID;
  card.open = false;
  const form = card.querySelector<HTMLFormElement>("form.semantic-form")!;
  form.dataset.ruleId = ruleID;
  form.dataset.previewId = ruleID;
  form.action = `/console/semantic-rules/${ruleID}`;
  form.querySelector<HTMLInputElement>("[name=display_name]")!.value = displayName;
  first.after(card);
  return card;
}

interface PreviewResponseOptions {
  kind?: PreviewKind;
  point?: Record<string, unknown>;
  points?: Array<Record<string, unknown>>;
  inputCount?: number;
  latestPoint?: Record<string, unknown>;
  ruleId?: string;
  displayName?: string;
  riseThreshold?: number;
  fallThreshold?: number;
}

function okPreviewResponse({
  kind = "numeric",
  point,
  points,
  inputCount,
  latestPoint,
  ruleId = "rule-01",
  displayName = "温度",
  riseThreshold,
  fallThreshold,
}: PreviewResponseOptions = {}): Response {
  const receivedAt = Number(point?.received_at ?? 1_000);
  const input = Number(point?.input ?? 24.8);
  const calibrated = Number(point?.calibrated ?? input);
  const basePoint = {
    received_at: receivedAt,
    input,
    input_min: Number(point?.input_min ?? input),
    input_max: Number(point?.input_max ?? input),
    calibrated,
    calibrated_min: Number(point?.calibrated_min ?? calibrated),
    calibrated_max: Number(point?.calibrated_max ?? calibrated),
    sample_count: 1,
  };
  const resolvedPoints = points ?? [{ ...basePoint, ...point }];
  const resolvedLatestPoint = latestPoint
    ? { ...basePoint, ...latestPoint }
    : resolvedPoints.at(-1);
  return new Response(
    JSON.stringify({
      calibration: {
        signal_ref: "sig_01",
        revision: 1,
        scale: 1,
        offset: 0,
        created_at: 1,
      },
      rules: [
        {
          rule_id: ruleId,
          display_name: displayName,
          kind,
          input_count: inputCount ?? resolvedPoints.length,
          plot_count: resolvedPoints.length,
          rise_threshold: riseThreshold,
          fall_threshold: fallThreshold,
          points: resolvedPoints,
          latest_point: resolvedLatestPoint,
        },
      ],
      window_start: resolvedPoints[0]?.plot_at ?? resolvedPoints[0]?.received_at ?? receivedAt,
      window_end: resolvedPoints.at(-1)?.plot_at ?? resolvedPoints.at(-1)?.received_at ?? receivedAt,
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  );
}

function previewResponse(receivedAt = 1000, input = 24.8): Response {
  return okPreviewResponse({ point: { received_at: receivedAt, input } });
}

function historySeriesResponse(
  latestValue: number | null,
  points: Array<Record<string, number>>,
  latestReceivedAt: number | null = 2_000,
): Response {
  return new Response(
    JSON.stringify({
      signal_ref: "sig_01",
      display_name: "温度",
      unit: "℃",
      value_type: "cumulative_counter",
      sample_count: points.reduce(
        (total, point) => total + Number(point.sample_count ?? 1),
        0,
      ),
      latest_received_at: latestReceivedAt,
      latest_value: latestValue,
      points,
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  );
}

function deferredResponse(): {
  promise: Promise<Response>;
  resolve: (response: Response) => void;
} {
  let resolve!: (response: Response) => void;
  const promise = new Promise<Response>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

function multiplePreviewResponse(
  rules: Array<Record<string, unknown>>,
  receivedAt = 1_000,
): Response {
  return new Response(
    JSON.stringify({
      calibration: {
        signal_ref: "sig_01",
        revision: 1,
        scale: 1,
        offset: 0,
        created_at: 1,
      },
      rules,
      window_start: receivedAt,
      window_end: receivedAt,
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  );
}

afterEach(() => {
  for (const toggle of document.querySelectorAll<HTMLButtonElement>(
    "[data-preview-toggle]",
  )) {
    if (toggle.getAttribute("aria-checked") === "true") toggle.click();
  }
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe("automatic mapping preview", () => {
  it("offers the saved rule and draft, then selects the draft explicitly", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const normal = document.querySelector<HTMLElement>(
      '[data-setting-panel="normal"]',
    )!;
    const alarm = document.querySelector<HTMLElement>(
      '[data-setting-panel="alarm"]',
    )!;
    alarm.hidden = true;
    const saved = normal.querySelector<HTMLDetailsElement>(
      ".semantic-rule-card",
    )!;
    const draft = normal.querySelector<HTMLDetailsElement>(
      ".semantic-rule-create",
    )!;
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(
      multiplePreviewResponse([
        {
          rule_id: "rule-01",
          display_name: "温度",
          kind: "numeric",
          input_count: 0,
          plot_count: 0,
          points: [],
        },
        {
          rule_id: "draft-normal",
          display_name: "新しい計測ルール",
          kind: "numeric",
          input_count: 0,
          plot_count: 0,
          points: [],
        },
      ]),
    ));
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    expect(Array.from(selector.options).map((option) => option.value)).toEqual([
      "rule-01",
      "draft-normal",
    ]);
    expect(Array.from(selector.options).map((option) => option.text)).toEqual([
      "温度",
      "新しい計測ルール",
    ]);
    selector.value = "draft-normal";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    expect(selector.value).toBe("draft-normal");
    expect(saved.open).toBe(false);
    expect(draft.open).toBe(true);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "新しい計測ルール",
      ),
    );

    const [, request] = fetchMock.mock.calls.at(-1) as [string, RequestInit];
    const requestRules = JSON.parse(String(request.body)).rules as Array<{
      rule_id: string;
    }>;
    expect(requestRules.map((rule) => rule.rule_id)).toContain("rule-01");
    expect(requestRules.map((rule) => rule.rule_id)).toContain("draft-normal");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps raw-only preview until the only alarm draft is opened", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const normal = document.querySelector<HTMLElement>(
      '[data-setting-panel="normal"]',
    )!;
    const alarm = document.querySelector<HTMLElement>(
      '[data-setting-panel="alarm"]',
    )!;
    normal.hidden = true;
    alarm.hidden = false;
    const draft = alarm.querySelector<HTMLDetailsElement>(
      ".semantic-rule-create",
    )!;
    draft.open = false;
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(
      multiplePreviewResponse([
        {
          rule_id: "rule-01",
          display_name: "温度",
          kind: "numeric",
          input_count: 1,
          plot_count: 1,
          points: [],
        },
        {
          rule_id: "draft-alarm",
          display_name: "新しい異常検知",
          kind: "alarm",
          input_count: 1,
          plot_count: 1,
          points: [],
        },
      ]),
    ));
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    expect(selector.disabled).toBe(false);
    expect(Array.from(selector.options).map((option) => option.value)).toEqual([
      "",
      "draft-alarm",
    ]);
    expect(Array.from(selector.options).map((option) => option.text)).toEqual([
      "ルールを選択",
      "新しい異常検知",
    ]);
    expect(selector.value).toBe("");
    expect(
      document.querySelector("[data-preview-rule-name]")?.textContent,
    ).toContain("選択中のルールはありません");
    draft.open = true;
    await vi.waitFor(() => expect(selector.value).toBe("draft-alarm"));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "新しい異常検知",
      ),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps raw data when the active target returns an error", async () => {
    installPreviewDOM();
    const normal = document.querySelector<HTMLElement>(
      '[data-setting-panel="normal"]',
    )!;
    const alarm = document.querySelector<HTMLElement>(
      '[data-setting-panel="alarm"]',
    )!;
    normal.hidden = true;
    alarm.hidden = false;
    const fetchMock = vi.fn().mockResolvedValue(
      multiplePreviewResponse([
        {
          rule_id: "rule-01",
          display_name: "温度",
          kind: "boolean",
          input_count: 1,
          plot_count: 1,
          points: [
            {
              received_at: 1_000,
              input: 12,
              input_min: 12,
              input_max: 12,
              calibrated: 99,
              calibrated_min: 99,
              calibrated_max: 99,
              active: true,
              active_samples: 1,
              sample_count: 1,
            },
          ],
        },
        {
          rule_id: "draft-alarm",
          display_name: "新しい異常検知",
          kind: "alarm",
          input_count: 1,
          plot_count: 1,
          error: "invalid detector",
          points: [
            {
              received_at: 1_000,
              input: 12,
              input_min: 12,
              input_max: 12,
              calibrated: 88,
              calibrated_min: 88,
              calibrated_max: 88,
              active: true,
              active_samples: 1,
              sample_count: 1,
            },
          ],
        },
      ]),
    );
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(request.body))).not.toHaveProperty("test_value");
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-accessible-summary]")
          ?.textContent,
      ).toContain("受信値は12から12"),
    );

    expect(
      document.querySelector("[data-preview-accessible-summary]")?.textContent,
    ).not.toContain("99");
    expect(document.querySelector(".chart-active-band")).toBeNull();
    expect(document.querySelector("[data-preview-test-result]")).toBeNull();
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("uses a temporary numeric draft to preview raw data before the first rule is saved", async () => {
    installNoRulePreviewDOM();
    const fetchMock = vi.fn().mockResolvedValue(previewResponse(1_000, 42));
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();

    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(request.body))).toMatchObject({
      signal_ref: "sig_01",
      calibration: { scale: 1, offset: 0 },
      rules: [
        {
          rule_id: "draft-raw",
          spec: { kind: "numeric" },
        },
      ],
    });
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-feed-state]")?.textContent,
      ).toBe("実データを表示中"),
    );
    document
      .querySelector<HTMLButtonElement>("[data-preview-toggle]")
      ?.click();
  });

  it("formats current values with the profile decimal-place setting", async () => {
    installPreviewDOM();
    const profile = document.createElement("form");
    profile.dataset.signalProfile = "";
    profile.innerHTML = `
      <select name="display_value_kind"><option value="numeric" selected>数値</option></select>
      <input name="decimal_places" value="1">
    `;
    document.body.append(profile);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(previewResponse(1_000, 42.45)));

    initializePreviews();

    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-current-value]")?.textContent,
      ).toBe("42.5"),
    );
    expect(
      document.querySelector("[data-source-current-value]")?.textContent,
    ).toBe("42.5");
    document
      .querySelector<HTMLButtonElement>("[data-preview-toggle]")
      ?.click();
  });

  it("updates the compact sensor header with the latest received value", async () => {
    installPreviewDOM();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(previewResponse()));

    initializePreviews();

    await vi.waitFor(() => {
      expect(
        document.querySelector("[data-source-current-value]")?.textContent,
      ).toBe("24.8");
    });
    expect(
      document
        .querySelector(".sensor-detail-latest")
        ?.getAttribute("data-source-value"),
    ).toBe("24.8");
    expect(
      document.querySelector("[data-source-current-received]")?.textContent,
    ).not.toBe("未受信");
  });

  it("uses the receipt-order latest point for current values while plotting by plot time", async () => {
    installPreviewDOM();
    const points = [
      {
        received_at: 500,
        plot_at: 1_000,
        input: 1,
        input_min: 1,
        input_max: 1,
        calibrated: 1,
        calibrated_min: 1,
        calibrated_max: 1,
        sample_count: 1,
      },
      {
        received_at: 500,
        plot_at: 2_000,
        input: 2,
        input_min: 2,
        input_max: 2,
        calibrated: 2,
        calibrated_min: 2,
        calibrated_max: 2,
        sample_count: 1,
      },
    ];
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        okPreviewResponse({
          points,
          latestPoint: {
            received_at: 900,
            plot_at: 1_500,
            input: 9,
            input_min: 9,
            input_max: 9,
            calibrated: 9,
            calibrated_min: 9,
            calibrated_max: 9,
          },
        }),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-current-value]")?.textContent).toBe(
        "9",
      ),
    );
    expect(document.querySelector("[data-source-current-value]")?.textContent).toBe(
      "9",
    );
    expect(
      document.querySelector<HTMLElement>("[data-preview-current-received]")
        ?.title,
    ).toBe(new Date(900).toLocaleString("ja-JP"));
    expect(document.querySelector<SVGPathElement>(".chart-line-raw")?.getAttribute("d"))
      .toMatch(/^M 72\.00 .* L 348\.00/);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("distinguishes newly received data from a successful check with no new data", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const fetchMock = vi
      .fn()
      .mockImplementationOnce(async () => previewResponse(1_000, 24.8))
      .mockImplementationOnce(async () => previewResponse(2_000, 25.1))
      .mockImplementation(async () => previewResponse(2_000, 25.1));
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-feed-state]")?.textContent,
      ).toBe("実データを表示中"),
    );

    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-feed-state]")?.textContent,
      ).toBe("新しいデータを受信"),
    );
    expect(
      document.querySelector("[data-preview-current-value]")?.textContent,
    ).toBe("25.1");

    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-feed-state]")?.textContent,
      ).toBe("新着なし"),
    );
    expect(
      document.querySelector("[data-preview-checked-at]")?.textContent,
    ).toMatch(/^確認 \d{2}:\d{2}:\d{2}$/);
  });

  it("shows that automatic reception checks are paused", async () => {
    installPreviewDOM();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(previewResponse()));

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-feed-state]")?.textContent,
      ).toBe("実データを表示中"),
    );
    document
      .querySelector<HTMLButtonElement>("[data-preview-toggle]")!
      .click();

    expect(
      document.querySelector("[data-preview-feed-state]")?.textContent,
    ).toBe("更新停止中");
  });

  it.each([
    ["numeric", { calibrated: 24.5 }, "24.5 ℃"],
    ["boolean", { active: true }, "ON"],
    [
      "cumulative_counter",
      { counter: 42, increment: 1 },
      "保存済み累積値を取得できません",
    ],
    ["alarm", { active: false }, "正常"],
    ["alarm", { active: true }, "異常"],
  ])("renders the selected %s outcome", async (kind, point, expected) => {
    installPreviewDOM();
    if (kind === "cumulative_counter") setSavedRuleKind(kind);
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        return Promise.resolve(
          pathname === "/api/v1/history/series"
            ? new Response("unavailable", { status: 503 })
            : okPreviewResponse({
                kind: kind as PreviewKind,
                point,
                displayName: "確認対象",
              }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-value]")?.textContent,
      ).toContain(expected),
    );

    expect(
      document.querySelector("[data-preview-accessible-summary]")?.textContent,
    ).toContain("確認対象");
    expect(
      document.querySelector("[data-preview-accessible-summary]")?.textContent,
    ).toContain(expected);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it.each(["http", "network"] as const)(
    "neutralizes auxiliary semantic output after a %s failure",
    async (failure) => {
      vi.useFakeTimers();
      installPreviewDOM();
      const first = okPreviewResponse({
        kind: "alarm",
        displayName: "ルールA",
        point: { active: true },
      });
      const fetchMock = vi.fn().mockResolvedValueOnce(first);
      if (failure === "http") {
        fetchMock.mockResolvedValueOnce(
          new Response(
            JSON.stringify({
              error: {
                code: "invalid_request",
                message: "入力内容を確認してください。",
                request_id: "req_02",
              },
            }),
            { status: 400, headers: { "Content-Type": "application/json" } },
          ),
        );
      } else {
        fetchMock.mockRejectedValueOnce(new Error("network unavailable"));
      }
      vi.stubGlobal("fetch", fetchMock);

      initializePreviews();
      await vi.waitFor(() => {
        expect(
          document.querySelector("[data-preview-accessible-summary]")?.textContent,
        ).toContain("ルールA");
      });

      document
        .querySelector<HTMLInputElement>('[name="rise_threshold"]')!
        .dispatchEvent(new Event("input", { bubbles: true }));
      await vi.advanceTimersByTimeAsync(300);
      await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => {
        expect(
          document.querySelector("[data-preview-accessible-summary]")?.textContent,
        ).not.toContain("ルールA");
        expect(
          document.querySelector("[data-preview-accessible-summary]")?.textContent,
        ).not.toContain("異常");
        expect(document.querySelector("[data-preview-test-result]")).toBeNull();
      });
      document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
    },
  );

  it("selects the first saved rule in the native preview selector", async () => {
    installPreviewDOM();
    const saved = document.querySelector<HTMLDetailsElement>(
      '[data-setting-panel="normal"] details.semantic-rule-card',
    )!;
    const draft = document.querySelector<HTMLDetailsElement>(
      '[data-setting-panel="normal"] details.semantic-rule-create',
    )!;
    saved.open = false;
    draft.open = true;
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(okPreviewResponse()));

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-name]")?.textContent,
      ).toContain("温度"),
    );
    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    expect(selector.disabled).toBe(false);
    expect(Array.from(selector.options).map((option) => option.value)).toEqual([
      "rule-01",
      "draft-normal",
    ]);
    expect(selector.value).toBe("rule-01");
    expect(saved.open).toBe(true);
    expect(draft.open).toBe(false);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("ignores queued restore toggles without ignoring a later direct draft open", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const saved = document.querySelector<HTMLDetailsElement>(
      '[data-setting-panel="normal"] details.semantic-rule-card',
    )!;
    const draft = document.querySelector<HTMLDetailsElement>(
      '[data-setting-panel="normal"] details.semantic-rule-create',
    )!;
    saved.open = false;
    draft.open = true;
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(
      multiplePreviewResponse([
        {
          rule_id: "rule-01",
          display_name: "温度",
          kind: "numeric",
          input_count: 1,
          plot_count: 1,
          points: [],
        },
        {
          rule_id: "draft-normal",
          display_name: "新しい計測ルール",
          kind: "numeric",
          input_count: 1,
          plot_count: 1,
          points: [],
        },
      ]),
    ));
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    saved.dispatchEvent(new Event("toggle"));
    draft.dispatchEvent(new Event("toggle"));
    await vi.advanceTimersByTimeAsync(300);
    expect(fetchMock).toHaveBeenCalledOnce();

    draft.open = true;
    draft.dispatchEvent(new Event("toggle"));
    await vi.waitFor(() =>
      expect(document.querySelector<HTMLSelectElement>("[data-preview-rule-select]")?.value)
        .toBe("draft-normal"),
    );
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps the native selector and saved rule cards in sync", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const panel = document.querySelector<HTMLElement>(
      '[data-setting-panel="normal"]',
    )!;
    const first = panel.querySelector<HTMLDetailsElement>(
      ".semantic-rule-card",
    )!;
    const second = first.cloneNode(true) as HTMLDetailsElement;
    second.dataset.ruleId = "rule-02";
    second.open = false;
    const secondForm = second.querySelector<HTMLFormElement>(
      "form.semantic-form",
    )!;
    secondForm.dataset.ruleId = "rule-02";
    secondForm.dataset.previewId = "rule-02";
    secondForm.action = "/console/semantic-rules/rule-02";
    secondForm.querySelector<HTMLInputElement>("[name=display_name]")!.value =
      "湿度";
    panel.prepend(second);
    const draft = panel.querySelector<HTMLDetailsElement>(
      ".semantic-rule-create",
    )!;
    const fetchMock = vi.fn().mockImplementation(() =>
      Promise.resolve(multiplePreviewResponse([
        {
          rule_id: "rule-01",
          display_name: "温度",
          kind: "numeric",
          input_count: 1,
          plot_count: 1,
          points: [],
        },
        {
          rule_id: "rule-02",
          display_name: "湿度",
          kind: "numeric",
          input_count: 1,
          plot_count: 1,
          points: [],
        },
      ])),
    );
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    expect(Array.from(selector.options).map((option) => option.value)).toEqual([
      "rule-02",
      "rule-01",
      "draft-normal",
    ]);
    selector.value = "rule-01";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    expect(selector.value).toBe("rule-01");
    expect(first.open).toBe(true);
    expect(second.open).toBe(false);
    expect(draft.open).toBe(false);
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-name]")?.textContent,
      ).toContain("温度"),
    );

    second.open = true;
    await vi.waitFor(() => expect(selector.value).toBe("rule-02"));
    expect(first.open).toBe(false);
    expect(second.open).toBe(true);
    expect(draft.open).toBe(false);
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-name]")?.textContent,
      ).toContain("湿度"),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("restores a tab's last selected saved rule after switching away and back", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const workspace = document.querySelector<HTMLElement>(
      ".sensor-setting-workspace",
    )!;
    const normal = document.querySelector<HTMLElement>(
      '[data-setting-panel="normal"]',
    )!;
    const alarm = document.querySelector<HTMLElement>(
      '[data-setting-panel="alarm"]',
    )!;
    const first = normal.querySelector<HTMLDetailsElement>(
      ".semantic-rule-card",
    )!;
    const second = addSavedRule("rule-02", "湿度");
    const alarmRule = second.cloneNode(true) as HTMLDetailsElement;
    alarmRule.dataset.ruleId = "rule-03";
    alarmRule.open = false;
    const alarmForm = alarmRule.querySelector<HTMLFormElement>(
      "form.semantic-form",
    )!;
    alarmForm.dataset.ruleId = "rule-03";
    alarmForm.dataset.previewId = "rule-03";
    alarmForm.action = "/console/semantic-rules/rule-03";
    alarmForm.querySelector<HTMLInputElement>("[name=display_name]")!.value =
      "高温";
    alarm.prepend(alarmRule);
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => Promise.resolve(
        multiplePreviewResponse([
          {
            rule_id: "rule-01",
            display_name: "温度",
            kind: "numeric",
            input_count: 1,
            plot_count: 1,
            points: [],
          },
          {
            rule_id: "rule-02",
            display_name: "湿度",
            kind: "numeric",
            input_count: 1,
            plot_count: 1,
            points: [],
          },
          {
            rule_id: "rule-03",
            display_name: "高温",
            kind: "alarm",
            input_count: 1,
            plot_count: 1,
            points: [],
          },
        ]),
      )),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "温度",
      ),
    );
    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    selector.value = "rule-02";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "湿度",
      ),
    );

    normal.hidden = true;
    alarm.hidden = false;
    workspace.dispatchEvent(new Event(SETTING_TAB_CHANGE_EVENT));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() => expect(selector.value).toBe("rule-03"));
    expect(alarmRule.open).toBe(true);

    alarm.hidden = true;
    normal.hidden = false;
    workspace.dispatchEvent(new Event(SETTING_TAB_CHANGE_EVENT));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() => expect(selector.value).toBe("rule-02"));
    expect(first.open).toBe(false);
    expect(second.open).toBe(true);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "湿度",
      ),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("updates saved and draft selector labels without changing selection", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const second = addSavedRule("rule-02", "湿度");
    const name = second.querySelector<HTMLInputElement>("[name=display_name]")!;
    const draftName = document.querySelector<HTMLInputElement>(
      '[data-preview-id="draft-normal"] [name="display_name"]',
    )!;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => Promise.resolve(
        multiplePreviewResponse([
          {
            rule_id: "rule-01",
            display_name: "温度",
            kind: "numeric",
            input_count: 1,
            plot_count: 1,
            points: [],
          },
          {
            rule_id: "rule-02",
            display_name: "湿度",
            kind: "numeric",
            input_count: 1,
            plot_count: 1,
            points: [],
          },
        ]),
      )),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector<HTMLSelectElement>("[data-preview-rule-select]")?.value)
        .toBe("rule-01"),
    );
    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    selector.value = "rule-02";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);

    name.value = "湿度（更新）";
    name.dispatchEvent(new Event("input", { bubbles: true }));
    expect(selector.value).toBe("rule-02");
    expect(selector.selectedOptions[0]?.text).toBe("湿度（更新）");

    name.value = "湿度（確定前）";
    name.dispatchEvent(new Event("change", { bubbles: true }));
    expect(selector.value).toBe("rule-02");
    expect(selector.selectedOptions[0]?.text).toBe("湿度（確定前）");

    draftName.value = "新しい計測ルール（編集）";
    draftName.dispatchEvent(new Event("input", { bubbles: true }));
    expect(selector.value).toBe("rule-02");
    expect(
      Array.from(selector.options).find((option) => option.value === "draft-normal")
        ?.text,
    ).toBe("新しい計測ルール（編集）");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps a no-saved tab raw-only until its draft is selected", async () => {
    vi.useFakeTimers();
    installNoRulePreviewDOM();
    const workspace = document.querySelector<HTMLElement>(
      ".sensor-setting-workspace",
    )!;
    const normal = document.querySelector<HTMLElement>(
      '[data-setting-panel="normal"]',
    )!;
    const alarm = document.querySelector<HTMLElement>(
      '[data-setting-panel="alarm"]',
    )!;
    const draft = document.querySelector<HTMLDetailsElement>(
      '[data-setting-panel="normal"] details.semantic-rule-create',
    )!;
    draft.open = false;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => Promise.resolve(
        okPreviewResponse({
          kind: "alarm",
          ruleId: "draft-normal",
          displayName: "新しい計測ルール",
          point: {
            input: 24,
            input_min: 23,
            input_max: 25,
            calibrated: 48,
            calibrated_min: 46,
            calibrated_max: 50,
            sample_count: 2,
            active: true,
            active_samples: 2,
          },
          riseThreshold: 30,
          fallThreshold: 20,
        }),
      )),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector(".chart-line-raw")).not.toBeNull(),
    );

    expect(document.querySelector(".chart-line-result")).toBeNull();
    expect(document.querySelector(".chart-range")).not.toBeNull();
    expect(document.querySelector(".chart-range-result")).toBeNull();
    expect(document.querySelector(".chart-latest-point")).toBeNull();
    expect(document.querySelector(".chart-threshold")).toBeNull();
    expect(document.querySelector(".chart-active-band")).toBeNull();
    expect(document.querySelector(".chart-line-counter")).toBeNull();
    expect(document.querySelector(".chart-increment")).toBeNull();
    expect(
      document.querySelector<HTMLElement>("[data-preview-result-legend]")
        ?.hidden,
    ).toBe(true);
    expect(
      document.querySelector<HTMLElement>("[data-preview-threshold-legend]")
        ?.hidden,
    ).toBe(true);
    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    expect(selector.disabled).toBe(false);
    expect(Array.from(selector.options).map((option) => option.value)).toEqual([
      "",
      "draft-normal",
    ]);
    expect(Array.from(selector.options).map((option) => option.text)).toEqual([
      "ルールを選択",
      "新しい計測ルール",
    ]);
    expect(selector.value).toBe("");
    expect(draft.open).toBe(false);
    expect(
      document.querySelector("[data-preview-rule-name]")?.textContent,
    ).toContain("選択中のルールはありません");
    selector.value = "draft-normal";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    expect(draft.open).toBe(true);
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "新しい計測ルール",
      ),
    );
    normal.hidden = true;
    alarm.hidden = false;
    workspace.dispatchEvent(new Event(SETTING_TAB_CHANGE_EVENT));
    await vi.advanceTimersByTimeAsync(300);
    expect(selector.value).toBe("");

    alarm.hidden = true;
    normal.hidden = false;
    workspace.dispatchEvent(new Event(SETTING_TAB_CHANGE_EVENT));
    await vi.advanceTimersByTimeAsync(300);
    expect(selector.value).toBe("draft-normal");
    expect(draft.open).toBe(true);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("disables the selector when the visible tab has no preview targets", async () => {
    installNoPreviewTargetDOM();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(okPreviewResponse()));

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector<HTMLSelectElement>("[data-preview-rule-select]")?.disabled)
        .toBe(true),
    );

    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    expect(Array.from(selector.options).map((option) => option.text)).toEqual([
      "選択できるルールなし",
    ]);
    expect(selector.value).toBe("");
    expect(
      document.querySelector("[data-preview-rule-name]")?.textContent,
    ).toContain("選択中のルールはありません");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("opens the first saved rule after a setting-tab change", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const workspace = document.querySelector<HTMLElement>(
      ".sensor-setting-workspace",
    )!;
    const normal = document.querySelector<HTMLElement>(
      '[data-setting-panel="normal"]',
    )!;
    const alarm = document.querySelector<HTMLElement>(
      '[data-setting-panel="alarm"]',
    )!;
    const saved = normal.querySelector<HTMLDetailsElement>(
      ".semantic-rule-card",
    )!;
    alarm.prepend(saved);
    saved.open = false;
    const alarmDraft = alarm.querySelector<HTMLDetailsElement>(
      ".semantic-rule-create",
    )!;
    alarmDraft.open = false;
    const fetchMock = vi.fn().mockImplementation(() =>
      Promise.resolve(okPreviewResponse()),
    );
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    normal.hidden = true;
    alarm.hidden = false;
    workspace.dispatchEvent(new Event(SETTING_TAB_CHANGE_EVENT));
    await vi.advanceTimersByTimeAsync(300);

    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-name]")?.textContent,
      ).toContain("温度"),
    );
    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    expect(selector.value).toBe("rule-01");
    expect(Array.from(selector.options).map((option) => option.value)).toEqual([
      "rule-01",
      "draft-alarm",
    ]);
    expect(saved.open).toBe(true);
    expect(alarmDraft.open).toBe(false);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("renders semantic overlays and legends for a successful selected rule", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        okPreviewResponse({
          kind: "alarm",
          point: {
            input: 24,
            input_min: 23,
            input_max: 25,
            calibrated: 48,
            calibrated_min: 46,
            calibrated_max: 50,
            sample_count: 2,
            active: true,
            active_samples: 2,
          },
          riseThreshold: 30,
          fallThreshold: 20,
        }),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector(".chart-line-result")).not.toBeNull(),
    );

    expect(document.querySelector(".chart-range-result")).not.toBeNull();
    expect(document.querySelector(".chart-latest-point")).not.toBeNull();
    expect(document.querySelectorAll(".chart-threshold")).toHaveLength(2);
    expect(document.querySelector(".chart-active-band")).not.toBeNull();
    expect(
      document.querySelector<HTMLElement>("[data-preview-result-legend]")
        ?.hidden,
    ).toBe(false);
    expect(
      document.querySelector<HTMLElement>("[data-preview-threshold-legend]")
        ?.hidden,
    ).toBe(false);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps cumulative previews to one raw line and latest marker", async () => {
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const points = [0, 10, 20, 10, 0].map((input, index) => ({
      received_at: 1_000 + index * 1_000,
      plot_at: 1_000 + index * 1_000,
      input,
      input_min: input,
      input_max: input,
      calibrated: input,
      calibrated_min: input,
      calibrated_max: input,
      active: true,
      active_samples: 1,
      sample_count: 1,
      counter: [0, 1, 1, 3, 3][index],
      increment: [0, 1, 0, 2, 0][index],
    }));
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        return Promise.resolve(
          pathname === "/api/v1/history/series"
            ? historySeriesResponse(null, [])
            : okPreviewResponse({
                kind: "cumulative_counter",
                points,
                riseThreshold: 15,
                fallThreshold: 5,
              }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector(".chart-line-raw")).not.toBeNull(),
    );
    expect(document.querySelector(".chart-line-result")).toBeNull();
    expect(document.querySelector(".chart-increment")).toBeNull();
    expect(document.querySelector(".chart-active-band")).toBeNull();
    expect(document.querySelector(".chart-line-counter")).toBeNull();
    expect(document.querySelector(".chart-counter-label")).toBeNull();
    expect(
      document.querySelector<HTMLElement>("[data-preview-result-legend]")
        ?.hidden,
    ).toBe(true);
    expect(document.querySelector(".chart-latest-point")).not.toBeNull();
    expect(document.querySelectorAll(".chart-threshold")).toHaveLength(2);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("starts saved counter history at page open and accumulates later changes", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    let historyCalls = 0;
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const pathname = new URL(String(input), "http://localhost").pathname;
      return Promise.resolve(
        pathname === "/api/v1/history/series"
          ? (() => {
              historyCalls += 1;
              if (historyCalls === 1) {
                return historySeriesResponse(90, [
                  { bucket_start: 8_000, minimum: 80, average: 80, maximum: 80, sample_count: 1 },
                  { bucket_start: 9_000, minimum: 90, average: 90, maximum: 90, sample_count: 1 },
                ]);
              }
              if (historyCalls === 2) {
                return historySeriesResponse(101, [
                  { bucket_start: 9_000, minimum: 90, average: 90, maximum: 90, sample_count: 1 },
                  { bucket_start: 10_500, minimum: 101, average: 101, maximum: 101, sample_count: 1 },
                ]);
              }
              return historySeriesResponse(102, [
                { bucket_start: 10_500, minimum: 101, average: 101, maximum: 101, sample_count: 1 },
                { bucket_start: 11_000, minimum: 102, average: 102, maximum: 102, sample_count: 1 },
              ]);
            })()
          : okPreviewResponse({
              kind: "cumulative_counter",
              point: { counter: 100, increment: 100 },
            }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "累積 90",
      ),
    );

    const counterPanel = document.querySelector<HTMLElement>("[data-preview-counter]");
    expect(counterPanel?.hidden).toBe(false);
    const counterSummary = document.querySelector<HTMLElement>(
      "[data-preview-counter-summary]",
    );
    expect(
      document.querySelector<SVGSVGElement>("[data-preview-counter-chart]")
        ?.getAttribute("aria-describedby"),
    ).toBe(counterSummary?.id);
    const counterPath = document.querySelector<SVGPathElement>(
      "[data-preview-counter-chart] .chart-line-raw",
    );
    expect(counterPath?.getAttribute("d")).toMatch(/^M 72\.00 /);
    expect(counterPath?.getAttribute("d")).not.toContain(" H ");
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("90");
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("1点");
    expect(
      document.querySelector("[data-preview-counter-chart] title")?.textContent,
    ).toContain("最新最大60点");
    const historyRequest = fetchMock.mock.calls
      .map(([input]) => new URL(String(input), "http://localhost"))
      .find((url) => url.pathname === "/api/v1/history/series");
    expect(historyRequest?.searchParams.get("rule_id")).toBe("rule-01");
    expect(Number(historyRequest?.searchParams.get("to"))).toBe(
      Number(historyRequest?.searchParams.get("from")) + 60_000,
    );

    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "累積 101",
      ),
    );
    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() => expect(historyCalls).toBe(3));
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "累積 102",
      ),
    );
    expect(
      document.querySelector<SVGPathElement>(
        "[data-preview-counter-chart] .chart-line-raw",
      )?.getAttribute("d")?.match(/ V /g),
    ).toHaveLength(2);
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("102");
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("3点");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("uses one baseline for an existing current despite out-of-order history rows", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        return Promise.resolve(
          pathname === "/api/v1/history/series"
            ? historySeriesResponse(101, [
                {
                  bucket_start: 9_500,
                  minimum: 102,
                  average: 102,
                  maximum: 102,
                  sample_count: 1,
                  last_value: 102,
                },
                {
                  bucket_start: 9_000,
                  minimum: 99,
                  average: 99,
                  maximum: 99,
                  sample_count: 1,
                  last_value: 99,
                },
              ], 9_500)
            : okPreviewResponse({
                kind: "cumulative_counter",
                point: { counter: 101, increment: 1 },
              }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "101",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("1点");
    expect(
      document.querySelector<SVGPathElement>(
        "[data-preview-counter-chart] .chart-line-raw",
      )?.getAttribute("d"),
    ).not.toContain(" H ");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("places an initial post-open current at its persisted receipt", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        return Promise.resolve(
          pathname === "/api/v1/history/series"
            ? historySeriesResponse(101, [
                {
                  bucket_start: 9_000,
                  minimum: 99,
                  average: 99,
                  maximum: 99,
                  sample_count: 1,
                  last_value: 99,
                },
              ], 11_000)
            : okPreviewResponse({
                kind: "cumulative_counter",
                point: { counter: 101, increment: 1 },
              }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "101",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("1点");
    const axisLabels = Array.from(
      document.querySelectorAll<SVGTextElement>(
        "[data-preview-counter-chart] .chart-axis-label",
      ),
    ).map((label) => label.textContent ?? "");
    expect(axisLabels.at(-2)).toBe(
      new Date(11_000).toLocaleTimeString("ja-JP", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      }),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("starts a saved-current session before a slow mapping resolves", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const delayedHistory = deferredResponse();
    const delayedMapping = deferredResponse();
    let historyCalls = 0;
    let mappingCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        if (pathname === "/api/v1/history/series") {
          historyCalls += 1;
          return delayedHistory.promise;
        }
        mappingCalls += 1;
        return delayedMapping.promise;
      }),
    );

    initializePreviews();
    await vi.waitFor(() => expect(historyCalls).toBe(1));
    expect(mappingCalls).toBe(1);

    vi.setSystemTime(12_000);
    delayedHistory.resolve(historySeriesResponse(101, [
      {
        bucket_start: 9_000,
        minimum: 99,
        average: 99,
        maximum: 99,
        sample_count: 1,
        last_value: 99,
      },
    ], 11_000));
    delayedMapping.resolve(
      okPreviewResponse({
        kind: "cumulative_counter",
        point: { counter: 101, increment: 1 },
      }),
    );

    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "101",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("1点");
    const axisLabels = Array.from(
      document.querySelectorAll<SVGTextElement>(
        "[data-preview-counter-chart] .chart-axis-label",
      ),
    ).map((label) => label.textContent ?? "");
    expect(axisLabels.at(-2)).toBe(
      new Date(11_000).toLocaleTimeString("ja-JP", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      }),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("records a saved current change despite a pre-session history bucket", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    let historyCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        if (pathname === "/api/v1/history/series") {
          historyCalls += 1;
          return Promise.resolve(
            historyCalls === 1
              ? historySeriesResponse(100, [], 9_000)
              : historySeriesResponse(101, [
                  {
                    bucket_start: 9_000,
                    minimum: 101,
                    average: 101,
                    maximum: 101,
                    sample_count: 1,
                    last_value: 101,
                  },
                ], 12_000),
          );
        }
        return Promise.resolve(
          okPreviewResponse({
            kind: "cumulative_counter",
            point: { counter: 101, increment: 1 },
          }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "100",
      ),
    );

    vi.setSystemTime(13_000);
    document
      .querySelector<HTMLFormElement>("form.semantic-form[data-rule-id]")
      ?.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() => expect(historyCalls).toBeGreaterThanOrEqual(2));
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "101",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("2点");
    expect(
      document.querySelector<SVGPathElement>(
        "[data-preview-counter-chart] .chart-line-raw",
      )?.getAttribute("d"),
    ).toMatch(/ H .* V /);
    const axisLabels = Array.from(
      document.querySelectorAll<SVGTextElement>(
        "[data-preview-counter-chart] .chart-axis-label",
      ),
    ).map((label) => label.textContent ?? "");
    expect(axisLabels.at(-1)).toBe(
      new Date(12_000).toLocaleTimeString("ja-JP", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      }),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("records a saved current change despite a late observed history bucket", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    let historyCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        if (pathname === "/api/v1/history/series") {
          historyCalls += 1;
          if (historyCalls === 1) {
            return Promise.resolve(historySeriesResponse(100, [], 9_000));
          }
          if (historyCalls === 2) {
            return Promise.resolve(
              historySeriesResponse(101, [
                {
                  bucket_start: 10_500,
                  minimum: 101,
                  average: 101,
                  maximum: 101,
                  sample_count: 1,
                  last_value: 101,
                },
              ], 10_500),
            );
          }
          return Promise.resolve(
            historySeriesResponse(102, [
              {
                bucket_start: 10_250,
                minimum: 102,
                average: 102,
                maximum: 102,
                sample_count: 1,
                last_value: 102,
              },
            ], 11_500),
          );
        }
        return Promise.resolve(
          okPreviewResponse({
            kind: "cumulative_counter",
            point: { counter: 102, increment: 1 },
          }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "100",
      ),
    );
    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "101",
      ),
    );
    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() => expect(historyCalls).toBe(3));
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "102",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("3点");
    expect(
      document.querySelector<SVGPathElement>(
        "[data-preview-counter-chart] .chart-line-raw",
      )?.getAttribute("d")?.match(/ V /g),
    ).toHaveLength(2);
    const axisLabels = Array.from(
      document.querySelectorAll<SVGTextElement>(
        "[data-preview-counter-chart] .chart-axis-label",
      ),
    ).map((label) => label.textContent ?? "");
    expect(axisLabels.at(-1)).toBe(
      new Date(11_500).toLocaleTimeString("ja-JP", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      }),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("drops the oldest saved counter point when a session reaches 61 points", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    let historyCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        if (pathname === "/api/v1/history/series") {
          historyCalls += 1;
          const value = historyCalls - 1;
          const receivedAt = historyCalls === 1
            ? 9_000
            : 10_000 + value * 1_000;
          return Promise.resolve(historySeriesResponse(value, [], receivedAt));
        }
        return Promise.resolve(
          okPreviewResponse({
            kind: "cumulative_counter",
            point: { counter: 60, increment: 1 },
          }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "0",
      ),
    );

    await vi.advanceTimersByTimeAsync(60_000);
    await vi.waitFor(() => expect(historyCalls).toBe(61));
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "60",
      ),
    );
    const path = document.querySelector<SVGPathElement>(
      "[data-preview-counter-chart] .chart-line-raw",
    )?.getAttribute("d");
    expect(path?.match(/ V /g)).toHaveLength(59);
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("60点");
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("最新最大60点");
    expect(
      document.querySelector("[data-preview-counter-chart] title")?.textContent,
    ).toContain("61点目から最古点");
    const axisLabels = Array.from(
      document.querySelectorAll<SVGTextElement>(
        "[data-preview-counter-chart] .chart-axis-label",
      ),
    ).map((label) => label.textContent ?? "");
    const timeLabel = (at: number) => new Date(at).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
    expect(axisLabels.at(-2)).toBe(timeLabel(11_000));
    expect(axisLabels.at(-2)).not.toBe(timeLabel(10_000));
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("resets saved counter history when the persisted rule changes", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    addSavedRule("rule-02", "湿度");
    const historyRuleIDs: string[] = [];
    let mappingCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = new URL(String(input), "http://localhost");
        if (url.pathname === "/api/v1/history/series") {
          const ruleID = url.searchParams.get("rule_id")!;
          historyRuleIDs.push(ruleID);
          return Promise.resolve(
            historySeriesResponse(ruleID === "rule-02" ? 200 : 100, []),
          );
        }
        mappingCalls += 1;
        return Promise.resolve(
          okPreviewResponse({
            kind: "cumulative_counter",
            ruleId: mappingCalls === 1 ? "rule-01" : "rule-02",
            point: { counter: mappingCalls === 1 ? 100 : 200, increment: 1 },
          }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "100",
      ),
    );

    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    selector.value = "rule-02";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "200",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).not.toContain("100");
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("1点");
    expect(historyRuleIDs).toEqual(["rule-01", "rule-02"]);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("does not retain an old session after a failed saved-rule switch", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const second = addSavedRule("rule-02", "湿度");
    const secondForm = second.querySelector<HTMLFormElement>(
      "form.semantic-form",
    )!;
    const historyRuleIDs: string[] = [];
    let mappingCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = new URL(String(input), "http://localhost");
        if (url.pathname === "/api/v1/history/series") {
          const ruleID = url.searchParams.get("rule_id")!;
          historyRuleIDs.push(ruleID);
          return Promise.resolve(
            historySeriesResponse(ruleID === "rule-02" ? 200 : 100, []),
          );
        }
        mappingCalls += 1;
        return Promise.resolve(
          mappingCalls === 2
            ? new Response("unavailable", { status: 503 })
            : okPreviewResponse({
                kind: "cumulative_counter",
                ruleId: mappingCalls === 1 ? "rule-01" : "rule-02",
                point: {
                  counter: mappingCalls === 1 ? 100 : 200,
                  increment: 1,
                },
              }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "100",
      ),
    );

    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    selector.value = "rule-02";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "判定結果を更新できません",
      ),
    );
    expect(historyRuleIDs).toContain("rule-02");
    expect(document.querySelector<HTMLElement>("[data-preview-counter]")?.hidden).toBe(
      true,
    );

    secondForm.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "200",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).not.toContain("100");
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("1点");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps the saved total and baseline after a quiet rolling history poll", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    let historyCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        if (pathname === "/api/v1/history/series") {
          historyCalls += 1;
          return Promise.resolve(
            historyCalls === 1
              ? historySeriesResponse(100, [])
              : historySeriesResponse(null, []),
          );
        }
        return Promise.resolve(
          okPreviewResponse({
            kind: "cumulative_counter",
            point: { counter: 100, increment: 0 },
          }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "100",
      ),
    );
    const chartPath = document.querySelector<SVGPathElement>(
      "[data-preview-counter-chart] .chart-line-raw",
    );

    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() => expect(historyCalls).toBe(2));
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "100",
      ),
    );
    expect(
      document.querySelector("[data-preview-counter-summary]")?.textContent,
    ).toContain("1点");
    expect(
      document.querySelector<SVGPathElement>(
        "[data-preview-counter-chart] .chart-line-raw",
      ),
    ).toBe(chartPath);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("uses the persisted current total rather than history-bucket averages", async () => {
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        return Promise.resolve(
          pathname === "/api/v1/history/series"
            ? historySeriesResponse(10, [
                {
                  bucket_start: 1_000,
                  minimum: 10,
                  average: 10,
                  maximum: 10,
                  sample_count: 1,
                  last_value: 10,
                },
                {
                  bucket_start: 1_500,
                  minimum: 10,
                  average: 50,
                  maximum: 90,
                  sample_count: 2,
                  last_value: 10,
                },
              ])
            : okPreviewResponse({
                kind: "cumulative_counter",
                point: { counter: 100, increment: 100 },
              }),
        );
      }),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "累積 10",
      ),
    );

    const path = document.querySelector<SVGPathElement>(
      "[data-preview-counter-chart] .chart-line-raw",
    )?.getAttribute("d");
    const coordinates = path?.match(/-?\d+(?:\.\d+)?/g)?.map(Number);
    expect(coordinates).toHaveLength(2);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps a slow counter history request alive across mapping polling and accepts it", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const delayedHistory = deferredResponse();
    let historyCalls = 0;
    let mappingCalls = 0;
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const pathname = new URL(String(input), "http://localhost").pathname;
      if (pathname === "/api/v1/history/series") {
        historyCalls += 1;
        return delayedHistory.promise;
      }
      mappingCalls += 1;
      return Promise.resolve(
        okPreviewResponse({
          kind: "cumulative_counter",
          point: { input: 24.8, counter: 100, increment: 3 },
        }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-current-value]")?.textContent).toBe(
        "24.8",
      ),
    );
    expect(historyCalls).toBe(1);
    expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
      "読み込み中",
    );

    await vi.advanceTimersByTimeAsync(3_000);
    expect(mappingCalls).toBeGreaterThan(1);
    expect(historyCalls).toBe(1);

    delayedHistory.resolve(historySeriesResponse(20, []));
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "累積 20",
      ),
    );
    expect(historyCalls).toBe(1);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps accepted legends and counter chart while mapping polling is pending", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const pendingMapping = deferredResponse();
    let mappingCalls = 0;
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const pathname = new URL(String(input), "http://localhost").pathname;
      if (pathname === "/api/v1/history/series") {
        return Promise.resolve(
          historySeriesResponse(591, [
            {
              bucket_start: 1_000,
              minimum: 591,
              average: 591,
              maximum: 591,
              sample_count: 1,
              last_value: 591,
            },
          ]),
        );
      }
      mappingCalls += 1;
      return mappingCalls === 1
        ? Promise.resolve(
            okPreviewResponse({
              kind: "cumulative_counter",
              point: {
                input: 24,
                input_min: 23,
                input_max: 25,
                calibrated: 48,
                calibrated_min: 46,
                calibrated_max: 50,
                counter: 100,
                increment: 3,
              },
            }),
          )
        : pendingMapping.promise;
    });
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector<HTMLElement>("[data-preview-result-legend]")
          ?.hidden,
      ).toBe(false),
    );
    await vi.waitFor(() =>
      expect(
        document.querySelector<SVGPathElement>(
          "[data-preview-counter-chart] .chart-line-raw",
        ),
      ).not.toBeNull(),
    );
    const chartPath = document
      .querySelector<SVGPathElement>("[data-preview-chart] .chart-line-raw")
      ?.getAttribute("d");
    const counterPath = document.querySelector<SVGPathElement>(
      "[data-preview-counter-chart] .chart-line-raw",
    );
    const counterPanel = document.querySelector<HTMLElement>(
      "[data-preview-counter]",
    );
    expect(counterPanel?.hidden).toBe(false);

    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() => expect(mappingCalls).toBe(2));
    expect(
      document.querySelector<HTMLElement>("[data-preview-result-legend]")
        ?.hidden,
    ).toBe(false);
    expect(counterPanel?.hidden).toBe(false);
    expect(
      document
        .querySelector<SVGPathElement>("[data-preview-chart] .chart-line-raw")
        ?.getAttribute("d"),
    ).toBe(chartPath);
    expect(
      document.querySelector<SVGPathElement>(
        "[data-preview-counter-chart] .chart-line-raw",
      ),
    ).toBe(counterPath);

    pendingMapping.resolve(
      okPreviewResponse({
        kind: "cumulative_counter",
        point: {
          input: 24,
          input_min: 23,
          input_max: 25,
          calibrated: 48,
          calibrated_min: 46,
          calibrated_max: 50,
          counter: 100,
          increment: 3,
        },
      }),
    );
    await vi.waitFor(() => expect(mappingCalls).toBe(2));
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps the saved counter chart DOM stable for an unchanged session poll", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(3_000);
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const advancingHistory = deferredResponse();
    let mappingCalls = 0;
    let historyCalls = 0;
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const pathname = new URL(String(input), "http://localhost").pathname;
      if (pathname === "/api/v1/history/series") {
        historyCalls += 1;
        return historyCalls === 1
          ? Promise.resolve(
              historySeriesResponse(100, [
                {
                  bucket_start: 1_000,
                  minimum: 100,
                  average: 100,
                  maximum: 100,
                  sample_count: 1,
                  last_value: 100,
                },
                {
                  bucket_start: 2_000,
                  minimum: 100,
                  average: 100,
                  maximum: 100,
                  sample_count: 1,
                  last_value: 100,
                },
              ]),
            )
          : advancingHistory.promise;
      }
      mappingCalls += 1;
      return Promise.resolve(
        okPreviewResponse({
          kind: "cumulative_counter",
          point: { counter: 100, increment: 0 },
        }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector<SVGPathElement>(
          "[data-preview-counter-chart] .chart-line-raw",
        ),
      ).not.toBeNull(),
    );
    const counterPath = document.querySelector<SVGPathElement>(
      "[data-preview-counter-chart] .chart-line-raw",
    );
    const initialPath = counterPath?.getAttribute("d");

    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() => expect(mappingCalls).toBeGreaterThan(1));
    await vi.waitFor(() => expect(historyCalls).toBe(2));
    advancingHistory.resolve(
      historySeriesResponse(100, [
        {
          bucket_start: 1_000,
          minimum: 100,
          average: 100,
          maximum: 100,
          sample_count: 1,
          last_value: 100,
        },
        {
          bucket_start: 2_000,
          minimum: 100,
          average: 100,
          maximum: 100,
          sample_count: 1,
          last_value: 100,
        },
        {
          bucket_start: 3_000,
          minimum: 100,
          average: 100,
          maximum: 100,
          sample_count: 1,
          last_value: 100,
        },
      ]),
    );
    await vi.waitFor(() =>
      expect(
        document.querySelector<SVGPathElement>(
          "[data-preview-counter-chart] .chart-line-raw",
        )?.getAttribute("d"),
      ).toBe(initialPath),
    );
    expect(
      document.querySelector<SVGPathElement>(
        "[data-preview-counter-chart] .chart-line-raw",
      ),
    ).toBe(counterPath);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps slow history from overwriting a failed mapping and reuses it after success", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const delayedHistory = deferredResponse();
    let mappingCalls = 0;
    let historyCalls = 0;
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const pathname = new URL(String(input), "http://localhost").pathname;
      if (pathname === "/api/v1/history/series") {
        historyCalls += 1;
        return historyCalls === 1
          ? delayedHistory.promise
          : Promise.resolve(historySeriesResponse(20, []));
      }
      mappingCalls += 1;
      return Promise.resolve(
        mappingCalls === 2
          ? new Response("unavailable", { status: 503 })
          : okPreviewResponse({
              kind: "cumulative_counter",
              point: { counter: 100, increment: 3 },
            }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(historyCalls).toBe(1));

    document
      .querySelector<HTMLFormElement>("form.semantic-form[data-rule-id]")!
      .dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
        "判定結果を更新できません",
      ),
    );
    const failureMessage = document.querySelector("[data-preview-message]")?.textContent;

    delayedHistory.resolve(historySeriesResponse(20, []));
    await delayedHistory.promise;
    await vi.advanceTimersByTimeAsync(1);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(document.querySelector("[data-preview-rule-name]")?.textContent).toContain(
      "判定結果を更新できません",
    );
    expect(document.querySelector("[data-preview-rule-value]")?.textContent).toBe("—");
    expect(document.querySelector("[data-preview-message]")?.textContent).toBe(
      failureMessage,
    );
    expect(document.querySelector<HTMLElement>("[data-preview-counter]")?.hidden).toBe(
      true,
    );

    document
      .querySelector<HTMLFormElement>("form.semantic-form[data-rule-id]")!
      .dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "累積 20",
      ),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps a valid empty persisted history distinct from an unavailable history", async () => {
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const pathname = new URL(String(input), "http://localhost").pathname;
      return Promise.resolve(
        pathname === "/api/v1/history/series"
          ? historySeriesResponse(null, [])
          : okPreviewResponse({
              kind: "cumulative_counter",
              point: { counter: 100, increment: 100 },
            }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "表示開始後の保存済み累積変化はありません",
      ),
    );
    expect(document.querySelector("[data-preview-rule-value]")?.textContent).not.toContain(
      "累積 100",
    );
    expect(document.querySelector("[data-preview-rule-detail]")?.textContent).not.toContain(
      "取得できません",
    );
    expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
      "表示開始後の保存済み累積変化はありません",
    );
    expect(document.querySelector("[data-preview-message]")?.textContent).toContain(
      "表示開始後の保存済み累積変化はありません",
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it.each(["HTTP", "network"] as const)(
    "shows persisted history as unavailable after a %s failure without replaying it",
    async (failure) => {
      installPreviewDOM();
      setSavedRuleKind("cumulative_counter");
      const fetchMock = vi.fn((input: RequestInfo | URL) => {
        const pathname = new URL(String(input), "http://localhost").pathname;
        if (pathname === "/api/v1/history/series") {
          if (failure === "HTTP") {
            return Promise.resolve(
              new Response("unavailable", { status: 503 }),
            );
          }
          return Promise.reject(new Error("network unavailable"));
        }
        return Promise.resolve(
          okPreviewResponse({
            kind: "cumulative_counter",
            point: { counter: 100, increment: 100 },
          }),
        );
      });
      vi.stubGlobal("fetch", fetchMock);

      initializePreviews();
      await vi.waitFor(() =>
        expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
          "取得できません",
        ),
      );
      expect(
        document.querySelector("[data-preview-counter-chart] .chart-empty-title")
          ?.textContent,
      ).toContain("取得できません");
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "保存済み累積値を取得できません",
      );
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).not.toContain(
        "累積 100",
      );
      expect(document.querySelector("[data-preview-message]")?.textContent).toContain(
        "取得できません",
      );
      expect(document.querySelector("[data-preview-message]")?.textContent).not.toContain(
        "別グラフで確認できます",
      );
      document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
    },
  );

  it("uses a cumulative draft after it is opened", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    setSavedRuleKind("cumulative_counter");
    const saved = document.querySelector<HTMLDetailsElement>(
      '[data-setting-panel="normal"] details.semantic-rule-card',
    )!;
    const draft = document.querySelector<HTMLDetailsElement>(
      '[data-setting-panel="normal"] details.semantic-rule-create',
    )!;
    draft.querySelector<HTMLInputElement>('[name="kind"]')!.value =
      "cumulative_counter";
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const pathname = new URL(String(input), "http://localhost").pathname;
      return Promise.resolve(
        pathname === "/api/v1/history/series"
          ? historySeriesResponse(100, [])
          : multiplePreviewResponse([
              {
                rule_id: "rule-01",
                display_name: "温度",
                kind: "cumulative_counter",
                input_count: 1,
                plot_count: 1,
                points: [{
                  received_at: 1_000,
                  input: 24.8,
                  input_min: 24.8,
                  input_max: 24.8,
                  calibrated: 24.8,
                  calibrated_min: 24.8,
                  calibrated_max: 24.8,
                  sample_count: 1,
                  counter: 100,
                  increment: 100,
                }],
              },
              {
                rule_id: "draft-normal",
                display_name: "新しい計測ルール",
                kind: "cumulative_counter",
                input_count: 1,
                plot_count: 1,
                points: [{
                  received_at: 1_000,
                  input: 24.8,
                  input_min: 24.8,
                  input_max: 24.8,
                  calibrated: 24.8,
                  calibrated_min: 24.8,
                  calibrated_max: 24.8,
                  sample_count: 1,
                  counter: 100,
                  increment: 100,
                }],
              },
            ]),
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-counter-summary]")?.textContent).toContain(
        "100",
      ),
    );
    draft.open = true;
    const selector = document.querySelector<HTMLSelectElement>(
      "[data-preview-rule-select]",
    )!;
    await vi.waitFor(() => expect(selector.value).toBe("draft-normal"));
    expect(saved.open).toBe(false);
    expect(draft.open).toBe(true);
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector("[data-preview-rule-value]")?.textContent).toContain(
        "直近60秒で +100",
      ),
    );
    expect(document.querySelector<HTMLElement>("[data-preview-counter]")?.hidden).toBe(
      true,
    );
    expect(
      fetchMock.mock.calls.some(([input]) =>
        new URL(String(input), "http://localhost").pathname ===
        "/api/v1/history/series",
      ),
    ).toBe(true);
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("shows result overlays when only the calibrated range changes", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        okPreviewResponse({
          kind: "numeric",
          point: {
            input: 10,
            input_min: 9,
            input_max: 11,
            calibrated: 10,
            calibrated_min: 8,
            calibrated_max: 12,
            sample_count: 2,
          },
        }),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector(".chart-line-result")).not.toBeNull(),
    );

    expect(
      document.querySelector<HTMLElement>("[data-preview-result-legend]")
        ?.hidden,
    ).toBe(false);
    expect(
      document.querySelector("[data-preview-message]")?.textContent,
    ).not.toContain("変換前後の値は同じです");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("uses the sensor boolean metadata for the raw preview step shape", async () => {
    installPreviewDOM();
    for (const form of document.querySelectorAll<HTMLFormElement>(
      "form.semantic-form",
    )) {
      form.dataset.booleanInput = "true";
    }
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        okPreviewResponse({
          points: [0, 1, 0].map((input, index) => ({
            received_at: (index + 1) * 1_000,
            input,
            input_min: input,
            input_max: input,
            calibrated: index + 4,
            calibrated_min: index + 4,
            calibrated_max: index + 4,
            sample_count: 1,
          })),
        }),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector(".chart-line-raw")).not.toBeNull(),
    );
    expect(
      document.querySelector<SVGPathElement>(".chart-line-raw")?.getAttribute("d"),
    ).toMatch(/ H .* V .* H .* V /);
    expect(
      Array.from(document.querySelectorAll(".chart-axis-label")).map(
        (label) => label.textContent,
      ),
    ).not.toContain("ON");
    expect(
      Array.from(document.querySelectorAll(".chart-axis-label")).some(
        (label) => Number(label.textContent) > 5.5,
      ),
    ).toBe(true);
    expect(document.querySelector("[data-preview-chart] title")?.textContent).toContain(
      "℃",
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("keeps a recent 20-second triangle readable inside the latest 60 seconds", async () => {
    installPreviewDOM();
    const points = [
      { received_at: 80_000, input: 0 },
      { received_at: 85_000, input: 10 },
      { received_at: 90_000, input: 20 },
      { received_at: 95_000, input: 10 },
      { received_at: 100_000, input: 0 },
    ].map((point) => ({
      received_at: point.received_at,
      input: point.input,
      input_min: point.input,
      input_max: point.input,
      calibrated: point.input,
      calibrated_min: point.input,
      calibrated_max: point.input,
      sample_count: 1,
    }));
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        okPreviewResponse({
          points,
          inputCount: 125,
          displayName: "三角波",
        }),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector(".chart-line-raw")).not.toBeNull(),
    );
    const path = document
      .querySelector<SVGPathElement>(".chart-line-raw")
      ?.getAttribute("d");
    expect(path).toMatch(/^M .* L .* L .* L .* L /);
    expect(
      document.querySelector<SVGSVGElement>("[data-preview-chart]")?.dataset
        .chartGeometry,
    ).toBe("compact");
    expect(document.querySelector("[data-preview-count]")?.textContent).toContain(
      "125件を評価",
    );
    expect(document.querySelector("[data-preview-count]")?.textContent).toContain(
      "5bucketを表示",
    );
    expect(
      document.querySelector("[data-preview-accessible-summary]")?.textContent,
    ).toContain("受信値は0から20");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("renders an error result when the selected rule fails", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        multiplePreviewResponse([
          {
            rule_id: "rule-01",
            display_name: "確認対象",
            kind: "alarm",
            input_count: 1,
            plot_count: 1,
            error: "invalid detector",
            points: [],
          },
        ]),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-name]")?.textContent,
      ).toContain("判定結果を更新できません"),
    );
    expect(
      document.querySelector("[data-preview-rule-value]")?.textContent,
    ).toBe("—");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("announces the selected error while keeping only raw reception visible", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        multiplePreviewResponse([
          {
            rule_id: "rule-01",
            display_name: "確認対象",
            kind: "alarm",
            input_count: 1,
            plot_count: 1,
            error: "invalid detector",
            points: [],
          },
          {
            rule_id: "rule-02",
            display_name: "別のルール",
            kind: "numeric",
            input_count: 1,
            plot_count: 1,
            points: [
              {
                received_at: 1_000,
                input: 24.8,
                input_min: 24.8,
                input_max: 24.8,
                calibrated: 99,
                calibrated_min: 99,
                calibrated_max: 99,
                sample_count: 1,
              },
            ],
          },
        ]),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-name]")?.textContent,
      ).toContain("確認対象"),
    );

    const cardName = document.querySelector("[data-preview-rule-name]")
      ?.textContent;
    const cardKind = document.querySelector("[data-preview-rule-kind]")
      ?.textContent;
    const cardDetail = document.querySelector("[data-preview-rule-detail]")
      ?.textContent;
    const summary = document.querySelector("[data-preview-accessible-summary]")
      ?.textContent;
    expect(cardName).toContain("判定結果を更新できません");
    expect(cardKind).toBe("異常検知");
    expect(cardDetail).toContain("受信値はそのまま確認できます");
    expect(summary).toContain("確認対象");
    expect(summary).toContain("異常検知");
    expect(summary).toContain("判定結果を更新できません");
    expect(summary).not.toContain("選択中のルールはありません");
    expect(summary).not.toContain("別のルール");
    expect(
      document.querySelector("[data-preview-message]")?.textContent,
    ).toContain("判定結果を更新できません");
    expect(
      document.querySelector("[data-preview-current-value]")?.textContent,
    ).toBe("24.8");
    expect(
      document.querySelector("[data-preview-rule-value]")?.textContent,
    ).toBe("—");
    expect(document.querySelector(".chart-line-result")).toBeNull();
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("renders a waiting result for an empty selected rule", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        okPreviewResponse({
          kind: "numeric",
          points: [],
          displayName: "確認対象",
        }),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-value]")?.textContent,
      ).toBe("受信待ち"),
    );
    expect(
      document.querySelector("[data-preview-accessible-summary]")?.textContent,
    ).toContain("確認対象は受信データを待っています。");
    expect(
      document.querySelector("[data-preview-feed-state]")?.textContent,
    ).toBe("受信待ち");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("renders a validation state when the preview request is rejected", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: {
              code: "invalid_request",
              message: "入力内容を確認してください。",
              request_id: "req_01",
            },
          }),
          { status: 400, headers: { "Content-Type": "application/json" } },
        ),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-rule-name]")?.textContent,
      ).toContain("設定内容を確認してください"),
    );
    expect(
      document.querySelector("[data-preview-rule-value]")?.textContent,
    ).toBe("—");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("retains the last raw value when the whole preview request fails", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(previewResponse(1_000, 24.8))
      .mockRejectedValueOnce(new Error("network unavailable"));
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-current-value]")?.textContent,
      ).toBe("24.8"),
    );

    document
      .querySelector<HTMLFormElement>("form.semantic-form")!
      .dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(
      document.querySelector("[data-preview-current-value]")?.textContent,
    ).toBe("24.8");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("debounces edits and sends the complete multi-rule request", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const fetchMock = vi.fn().mockImplementation(async () => previewResponse());
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    const scale = document.querySelector<HTMLInputElement>(
      'form[action$="/calibration"] [name="scale"]',
    )!;
    scale.value = "2";
    scale.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(299);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));

    const [, request] = fetchMock.mock.calls[1] as [string, RequestInit];
    expect(request.headers).toMatchObject({
      "X-CSRF-Token": "csrf-test",
    });
    expect(JSON.parse(String(request.body))).toMatchObject({
      signal_ref: "sig_01",
      calibration: { scale: 2, offset: 0 },
      rules: [
        {
          rule_id: "rule-01",
          display_name: "温度",
          spec: {
            kind: "numeric",
            detector: {
              mode: "",
              rise_threshold: 0,
              fall_threshold: 0,
              rise_debounce_ms: 0,
              fall_debounce_ms: 0,
            },
            trigger: "",
          },
        },
      ],
    });
  });

  it("aborts the previous request before refreshing", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    let firstSignal: AbortSignal | undefined;
    const fetchMock = vi
      .fn()
      .mockImplementationOnce((_url: string, request: RequestInit) => {
        firstSignal = request.signal ?? undefined;
        return new Promise<Response>(() => {});
      })
      .mockImplementation(async () => previewResponse());
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    document
      .querySelector<HTMLInputElement>('[name="rise_threshold"]')!
      .dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);

    expect(firstSignal?.aborted).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("marks the invalid field returned by the server", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: {
              code: "invalid_request",
              message: "入力内容を確認してください。",
              field: "rise_threshold",
              request_id: "req_01",
            },
          }),
          { status: 400, headers: { "Content-Type": "application/json" } },
        ),
      ),
    );

    initializePreviews();

    await vi.waitFor(() =>
      expect(
        document
          .querySelector('[name="rise_threshold"]')
          ?.getAttribute("aria-invalid"),
      ).toBe("true"),
    );
    expect(
      document.querySelector("[data-preview-message]")?.textContent,
    ).toContain("立ち上がりしきい値を確認してください");
    const invalidField = document.querySelector<HTMLInputElement>(
      '[name="rise_threshold"]',
    )!;
    const error = invalidField
      .closest("label")
      ?.querySelector<HTMLElement>(".field-error");
    expect(error?.textContent).toContain(
      "立ち上がりしきい値を確認してください",
    );
    expect(invalidField.getAttribute("aria-describedby")).toBe(error?.id);
  });

  it("clears an inline field error after the preview becomes valid", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const invalidResponse = new Response(
      JSON.stringify({
        error: {
          code: "invalid_request",
          message: "入力内容を確認してください。",
          field: "rise_threshold",
          request_id: "req_01",
        },
      }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
    vi.stubGlobal(
      "fetch",
      vi.fn()
        .mockResolvedValueOnce(invalidResponse)
        .mockImplementation(async () => previewResponse()),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(document.querySelector(".field-error")).not.toBeNull(),
    );

    document
      .querySelector<HTMLInputElement>('[name="rise_threshold"]')!
      .dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(document.querySelector(".field-error")).toBeNull(),
    );
    expect(
      document
        .querySelector('[name="rise_threshold"]')
        ?.hasAttribute("aria-invalid"),
    ).toBe(false);
  });

  it("explains when the received and converted values overlap", async () => {
    installPreviewDOM();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(previewResponse()));

    initializePreviews();

    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-message]")?.textContent,
      ).toContain("変換前後の値は同じです"),
    );
    expect(
      document.querySelector<HTMLElement>("[data-preview-result-legend]")
        ?.hidden,
    ).toBe(true);
  });

  it("lets the operator pause periodic preview refreshes", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const fetchMock = vi.fn().mockResolvedValue(previewResponse());
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    document
      .querySelector<HTMLButtonElement>("[data-preview-toggle]")!
      .click();
    await vi.advanceTimersByTimeAsync(5_000);

    expect(fetchMock).toHaveBeenCalledOnce();
    expect(
      document.querySelector("[data-preview-toggle]")?.textContent,
    ).toBe("自動更新");
    expect(
      document
        .querySelector("[data-preview-toggle]")
        ?.getAttribute("aria-checked"),
    ).toBe("false");
  });

  it("provides a readable summary of the chart", async () => {
    installPreviewDOM();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(previewResponse()));

    initializePreviews();

    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-accessible-summary]")
          ?.textContent,
      ).toContain("受信値は24.8から24.8"),
    );
    expect(
      document.querySelector("[data-preview-accessible-summary]")
        ?.textContent,
    ).toContain("1件");
  });

  it("updates the SVG-described chart summary after preview success and failure", async () => {
    vi.useFakeTimers();
    installPreviewDOM();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(previewResponse())
      .mockResolvedValueOnce(
        new Response("unavailable", { status: 503 }),
      );
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    const chart = document.querySelector<SVGSVGElement>("[data-preview-chart]")!;
    const summaryID = chart.getAttribute("aria-describedby");
    expect(summaryID).toBe("sensor-preview-chart-summary");
    const summary = document.getElementById(summaryID!)!;
    expect(summary.hasAttribute("data-preview-accessible-summary")).toBe(true);
    await vi.waitFor(() => expect(summary.textContent).toContain("受信値は24.8から24.8"));

    document
      .querySelector<HTMLFormElement>("form.semantic-form")!
      .dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    await vi.waitFor(() =>
      expect(summary.textContent).toContain("判定結果を更新できません"),
    );
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

  it("includes raw and calibrated ranges in a successful numeric summary", async () => {
    installPreviewDOM();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        okPreviewResponse({
          kind: "numeric",
          displayName: "確認対象",
          points: [
            {
              received_at: 1_000,
              input: 10,
              input_min: 9,
              input_max: 11,
              calibrated: 20,
              calibrated_min: 18,
              calibrated_max: 22,
              sample_count: 1,
            },
            {
              received_at: 2_000,
              input: 12,
              input_min: 11,
              input_max: 13,
              calibrated: 24,
              calibrated_min: 23,
              calibrated_max: 25,
              sample_count: 1,
            },
          ],
        }),
      ),
    );

    initializePreviews();
    await vi.waitFor(() =>
      expect(
        document.querySelector("[data-preview-accessible-summary]")
          ?.textContent,
      ).toContain("受信値は9から13"),
    );
    const summary = document.querySelector("[data-preview-accessible-summary]")
      ?.textContent;
    expect(summary).toContain("補正後は18から25");
    expect(summary).toContain("最新の補正後は24 ℃");
    expect(summary).toContain("確認対象");
    expect(summary).toContain("測定値");
    expect(summary).toContain("2件");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });
});
