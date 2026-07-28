import { afterEach, describe, expect, it, vi } from "vitest";
import { initializePreviews } from "../../src/preview";

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
      <section data-setting-simulation data-signal-ref="sig_01">
        <button type="button" role="switch" aria-checked="true"
          data-preview-toggle>自動更新</button>
        <input name="preview_test_value">
        <span data-preview-test-result></span>
        <span data-preview-range></span>
        <span data-preview-count></span>
        <span data-preview-message></span>
        <span data-preview-feed-state>受信データを確認中</span>
        <span data-preview-checked-at></span>
        <span data-preview-accessible-summary></span>
        <strong data-preview-current-value></strong>
        <span data-preview-current-received></span>
        <svg data-preview-chart></svg>
        <dd data-preview-rule-name>選択中のルールはありません</dd>
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

function previewResponse(receivedAt = 1000, input = 24.8): Response {
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
          rule_id: "rule-01",
          display_name: "温度",
          kind: "numeric",
          input_count: 1,
          plot_count: 1,
          points: [
            {
              received_at: receivedAt,
              input,
              input_min: input,
              input_max: input,
              calibrated: input,
              calibrated_min: input,
              calibrated_max: input,
              sample_count: 1,
            },
          ],
        },
      ],
      window_start: receivedAt,
      window_end: receivedAt,
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  );
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
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe("automatic mapping preview", () => {
  it("uses the stable normal draft ID when its target is open", async () => {
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
    saved.open = false;
    draft.open = true;
    const fetchMock = vi.fn().mockResolvedValue(
      multiplePreviewResponse([
        {
          rule_id: "draft-normal",
          display_name: "新しい計測ルール",
          kind: "numeric",
          input_count: 0,
          plot_count: 0,
          points: [],
        },
      ]),
    );
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    const requestRules = JSON.parse(String(request.body)).rules as Array<{
      rule_id: string;
    }>;
    expect(requestRules.map((rule) => rule.rule_id)).toContain("draft-normal");
    expect(requestRules.map((rule) => rule.rule_id)).not.toContain("draft-1");
    document.querySelector<HTMLButtonElement>("[data-preview-toggle]")?.click();
  });

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
    const fetchMock = vi.fn().mockResolvedValue(
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
    );
    vi.stubGlobal("fetch", fetchMock);

    initializePreviews();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    const requestRules = JSON.parse(String(request.body)).rules as Array<{
      rule_id: string;
    }>;
    expect(requestRules.map((rule) => rule.rule_id)).toContain("draft-alarm");
    expect(requestRules.map((rule) => rule.rule_id)).not.toContain(
      "draft-normal",
    );
    expect(
      document.querySelector("[data-preview-rule-name]")?.textContent,
    ).toContain("選択中のルールはありません");
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
    const testInput = document.querySelector<HTMLInputElement>(
      '[name="preview_test_value"]',
    )!;
    testInput.value = "7";
    const fetchMock = vi.fn().mockResolvedValue(
      multiplePreviewResponse([
        {
          rule_id: "rule-01",
          display_name: "温度",
          kind: "boolean",
          input_count: 1,
          plot_count: 1,
          test_result: {
            emitted: true,
            boolean: true,
            calibrated: 99,
          },
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
    expect(JSON.parse(String(request.body)).test_value).toBe(7);
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
    expect(
      document.querySelector("[data-preview-test-result]")?.textContent,
    ).toBe("値を入力すると結果を確認できます");
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
    scale.dispatchEvent(new Event("input"));
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
      .querySelector<HTMLInputElement>('[name="preview_test_value"]')!
      .dispatchEvent(new Event("input"));
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
      .dispatchEvent(new Event("input"));
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
});
