import type { components } from "./generated/edge-api";
import {
  createMappingPreview,
  type MappingPreviewRequest,
  type MappingPreviewResponse,
} from "./api";
import {
  formField,
  numericFormField,
  query,
  queryAll,
} from "./dom";
import { csrfToken, SETTING_TAB_CHANGE_EVENT } from "./shell";
import {
  definitionSpec,
  ruleSpec,
  type SemanticKind,
} from "./semantic";

type PreviewBody = components["schemas"]["PreviewBody"];
type PreviewPoint = components["schemas"]["PreviewPoint"];
type SemanticRulePreview = components["schemas"]["SemanticRulePreview"];

interface PreviewSelection {
  raw: PreviewBody | null;
  selected: SemanticRulePreview | null;
}

interface RuleOutcome {
  value: string;
  detail: string;
  alarm: boolean;
}

const kindLabels: Record<SemanticKind, string> = {
  numeric: "測定値",
  boolean: "ON / OFF",
  cumulative_counter: "累積値",
  alarm: "異常検知",
};

const svgNamespace = "http://www.w3.org/2000/svg";

function addSVG<K extends keyof SVGElementTagNameMap>(
  parent: SVGElement,
  name: K,
  attributes: Record<string, string | number> = {},
): SVGElementTagNameMap[K] {
  const element = document.createElementNS(svgNamespace, name);
  for (const [key, value] of Object.entries(attributes)) {
    element.setAttribute(key, String(value));
  }
  parent.appendChild(element);
  return element;
}

function isFiniteNumber(value: unknown): value is number {
  return Number.isFinite(Number(value));
}

function formatNumber(value: unknown): string {
  return Number(value).toLocaleString("ja-JP", {
    maximumFractionDigits: 3,
  });
}

function formatCurrentValue(
  value: number,
  valueKind?: string,
  decimalPlaces?: number,
): string {
  if (valueKind === "boolean") {
    return Number(value) === 0 ? "OFF" : "ON";
  }
  if (!Number.isInteger(decimalPlaces)) return formatNumber(value);
  const digits = Math.min(6, Math.max(0, Number(decimalPlaces)));
  return Number(value).toLocaleString("ja-JP", {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}

function formatDuration(start: number, end: number): string {
  const milliseconds = Math.max(0, end - start);
  if (milliseconds < 60_000) {
    return `${Math.max(1, Math.round(milliseconds / 1000))}秒`;
  }
  return `${Math.max(1, Math.round(milliseconds / 60_000))}分`;
}

function setText(element: HTMLElement, value: string): void {
  if (element.textContent !== value) element.textContent = value;
}

function clearFieldErrors(panel: HTMLElement): void {
  for (const error of queryAll<HTMLElement>(".field-error", panel)) {
    error.remove();
  }
  for (const field of queryAll<HTMLElement>('[aria-invalid="true"]', panel)) {
    field.removeAttribute("aria-invalid");
    const describedBy = (field.getAttribute("aria-describedby") ?? "")
      .split(/\s+/)
      .filter((id) => id && !id.startsWith("preview-field-error-"));
    if (describedBy.length) {
      field.setAttribute("aria-describedby", describedBy.join(" "));
    } else {
      field.removeAttribute("aria-describedby");
    }
  }
}

function showFieldError(
  field: HTMLElement,
  label: string,
): void {
  field.setAttribute("aria-invalid", "true");
  const wrapper = field.closest("label");
  if (!wrapper) return;
  const error = document.createElement("small");
  error.className = "field-error";
  error.id = `preview-field-error-${field.getAttribute("name") ?? "field"}`;
  error.textContent = `${label}を確認してください。`;
  wrapper.append(error);
  const describedBy = new Set(
    (field.getAttribute("aria-describedby") ?? "")
      .split(/\s+/)
      .filter(Boolean),
  );
  describedBy.add(error.id);
  field.setAttribute("aria-describedby", Array.from(describedBy).join(" "));
}

function kindLabel(kind: SemanticKind): string {
  return kindLabels[kind];
}

function latestRuleOutcome(
  payload: PreviewBody,
  unit: string,
): RuleOutcome {
  const latest = payload.points?.at(-1);
  if (!latest) {
    return {
      value: "受信待ち",
      detail: "受信データを待っています。",
      alarm: false,
    };
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
        detail:
          Number(latest.increment ?? 0) > 0
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
      none: [
        "選択中のルールはありません",
        "—",
        "ルールを開くと判定結果を確認できます。",
      ],
      invalid: [
        "設定内容を確認してください",
        "—",
        "入力項目を修正してください。",
      ],
      error: [
        "判定結果を更新できません",
        "—",
        "受信値はそのまま確認できます。",
      ],
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

function previewWindow(
  payload: PreviewBody,
  points: PreviewPoint[],
): { start: number; end: number } {
  const now = Date.now();
  return {
    start: payload.window_start ?? points[0]?.received_at ?? now,
    end: payload.window_end ?? points.at(-1)?.received_at ?? now,
  };
}

function renderEmptyChart(
  svg: SVGSVGElement,
  payload: PreviewBody,
): void {
  const title = addSVG(svg, "text", {
    x: 380,
    y: 122,
    "text-anchor": "middle",
    class: "chart-empty-title",
  });
  title.textContent = payload.error
    ? "このルールでは受信値を判定できません"
    : "まだ受信データがありません";
  const hint = addSVG(svg, "text", {
    x: 380,
    y: 148,
    "text-anchor": "middle",
    class: "chart-empty-hint",
  });
  hint.textContent = payload.error
    ? "入力値の補正と判定条件を確認してください"
    : "試す値を入力して、設定結果を確認できます";
}

function renderPreviewChart(svg: SVGSVGElement, payload: PreviewBody): void {
  svg.replaceChildren();
  const points = payload.points ?? [];
  const width = 760;
  const height = 260;
  const left = 58;
  const right = 18;
  const top = 18;
  const bottom = 42;
  const plotWidth = width - left - right;
  const plotHeight = height - top - bottom;
  if (!points.length) {
    renderEmptyChart(svg, payload);
    return;
  }

  const values: number[] = [];
  for (const point of points) {
    for (const value of [
      point.input_min,
      point.input_max,
      point.calibrated_min,
      point.calibrated_max,
    ]) {
      if (isFiniteNumber(value)) values.push(Number(value));
    }
  }
  if (isFiniteNumber(payload.rise_threshold)) {
    values.push(payload.rise_threshold);
  }
  if (isFiniteNumber(payload.fall_threshold)) {
    values.push(payload.fall_threshold);
  }
  let minValue = Math.min(...values);
  let maxValue = Math.max(...values);
  if (minValue === maxValue) {
    const padding = Math.max(1, Math.abs(minValue) * 0.1);
    minValue -= padding;
    maxValue += padding;
  } else {
    const padding = (maxValue - minValue) * 0.08;
    minValue -= padding;
    maxValue += padding;
  }

  const firstReceivedAt = points[0].received_at;
  const lastReceivedAt = points.at(-1)?.received_at ?? firstReceivedAt;
  const x = (index: number): number => {
    if (points.length === 1) return left + plotWidth / 2;
    const point = points[index];
    if (lastReceivedAt > firstReceivedAt) {
      return (
        left +
        ((point.received_at - firstReceivedAt) * plotWidth) /
          (lastReceivedAt - firstReceivedAt)
      );
    }
    return left + (index * plotWidth) / (points.length - 1);
  };
  const y = (value: number): number =>
    top + ((maxValue - value) * plotHeight) / (maxValue - minValue);

  for (let index = 0; index <= 4; index += 1) {
    const gridY = top + (index * plotHeight) / 4;
    addSVG(svg, "line", {
      x1: left,
      x2: width - right,
      y1: gridY,
      y2: gridY,
      class: "chart-grid",
    });
    const label = addSVG(svg, "text", {
      x: left - 9,
      y: gridY + 4,
      "text-anchor": "end",
      class: "chart-axis-label",
    });
    label.textContent = formatNumber(
      maxValue - (index * (maxValue - minValue)) / 4,
    );
  }

  const drawThreshold = (
    value: number | undefined,
    labelText: string,
  ): void => {
    if (!isFiniteNumber(value)) return;
    const thresholdY = y(value);
    addSVG(svg, "line", {
      x1: left,
      x2: width - right,
      y1: thresholdY,
      y2: thresholdY,
      class: "chart-threshold",
    });
    const label = addSVG(svg, "text", {
      x: width - right - 4,
      y: thresholdY - 6,
      "text-anchor": "end",
      class: "chart-threshold-label",
    });
    label.textContent = `${labelText} ${formatNumber(value)}`;
  };
  drawThreshold(payload.rise_threshold, "立上り");
  drawThreshold(payload.fall_threshold, "立下り");

  points.forEach((point, index) => {
    if (point.sample_count > 1) {
      addSVG(svg, "line", {
        x1: x(index),
        x2: x(index),
        y1: y(point.input_min),
        y2: y(point.input_max),
        class: "chart-range",
      });
      addSVG(svg, "line", {
        x1: x(index) + 2,
        x2: x(index) + 2,
        y1: y(point.calibrated_min),
        y2: y(point.calibrated_max),
        class: "chart-range-result",
      });
    }
    if (payload.kind !== "numeric") {
      const ratio = point.sample_count
        ? Number(point.active_samples ?? 0) / point.sample_count
        : 0;
      if (ratio > 0) {
        addSVG(svg, "rect", {
          x:
            x(index) -
            Math.max(1, plotWidth / Math.max(points.length, 1) / 2),
          y: top,
          width: Math.max(2, plotWidth / Math.max(points.length, 1)),
          height: plotHeight,
          class: "chart-active-band",
          opacity: Math.max(0.12, ratio * 0.24),
        });
      }
    }
  });

  const path = (field: "input" | "calibrated"): string =>
    points
      .map(
        (point, index) =>
          `${index === 0 ? "M" : "L"} ${x(index).toFixed(2)} ` +
          `${y(point[field]).toFixed(2)}`,
      )
      .join(" ");
  addSVG(svg, "path", {
    d: path("input"),
    class: "chart-line chart-line-raw",
  });
  addSVG(svg, "path", {
    d: path("calibrated"),
    class: "chart-line chart-line-result",
  });
  const latestPoint = points.at(-1);
  if (latestPoint) {
    addSVG(svg, "circle", {
      cx: x(points.length - 1),
      cy: y(latestPoint.calibrated),
      r: 5,
      class: "chart-latest-point",
    });
    const latestLabel = addSVG(svg, "text", {
      x: Math.min(width - right - 4, x(points.length - 1) - 8),
      y: Math.max(top + 13, y(latestPoint.calibrated) - 10),
      "text-anchor": "end",
      class: "chart-latest-label",
    });
    latestLabel.textContent = "最新";
  }

  if (payload.kind === "cumulative_counter") {
    const maxIncrement = Math.max(
      1,
      ...points.map((point) => Number(point.increment ?? 0)),
    );
    points.forEach((point, index) => {
      const increment = Number(point.increment ?? 0);
      if (!increment) return;
      const barHeight = Math.max(3, (increment / maxIncrement) * 34);
      addSVG(svg, "rect", {
        x: x(index) - 2,
        y: top + plotHeight - barHeight,
        width: 4,
        height: barHeight,
        class: "chart-increment",
      });
    });
    const maxCounter = Math.max(
      1,
      ...points.map((point) => Number(point.counter ?? 0)),
    );
    const counterY = (value: number | undefined): number =>
      top + ((maxCounter - Number(value ?? 0)) * plotHeight) / maxCounter;
    const counterPath = points
      .map(
        (point, index) =>
          `${index === 0 ? "M" : "L"} ${x(index).toFixed(2)} ` +
          `${counterY(point.counter).toFixed(2)}`,
      )
      .join(" ");
    addSVG(svg, "path", {
      d: counterPath,
      class: "chart-line chart-line-counter",
    });
    const latestCounter = points.at(-1)?.counter;
    const counterLabel = addSVG(svg, "text", {
      x: width - right - 4,
      y: counterY(latestCounter) - 7,
      "text-anchor": "end",
      class: "chart-counter-label",
    });
    counterLabel.textContent = `累積 ${formatNumber(latestCounter ?? 0)}`;
  }

  const window = previewWindow(payload, points);
  const start = addSVG(svg, "text", {
    x: left,
    y: height - 14,
    class: "chart-axis-label",
  });
  start.textContent = new Date(window.start).toLocaleTimeString("ja-JP", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  const end = addSVG(svg, "text", {
    x: width - right,
    y: height - 14,
    "text-anchor": "end",
    class: "chart-axis-label",
  });
  end.textContent = new Date(window.end).toLocaleTimeString("ja-JP", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function isMultipleRulePreview(
  response: MappingPreviewResponse,
): response is components["schemas"]["MultipleRuleMappingPreview"] {
  return "rules" in response;
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

function rawOnlyPreview(payload: PreviewBody): PreviewBody {
  return {
    ...payload,
    kind: "numeric",
    test_result: undefined,
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

function buildRequest(
  signalRef: string,
  forms: HTMLFormElement[],
  calibrationForm: HTMLFormElement | null,
  multipleRules: boolean,
  testInput: HTMLInputElement | null,
  activeID?: string,
): MappingPreviewRequest {
  const body: MappingPreviewRequest = { signal_ref: signalRef };
  const firstForm = forms[0];
  if (multipleRules) {
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
    if (!rules.length) {
      rules.push({
        rule_id: "draft-raw",
        display_name: "受信値",
        spec: { kind: "numeric" },
      });
    }
    if (rules.length) {
      body.calibration = {
        scale: calibrationForm
          ? numericFormField(calibrationForm, "scale")
          : 1,
        offset: calibrationForm
          ? numericFormField(calibrationForm, "offset")
          : 0,
      };
      body.rules = rules;
    }
  } else if (firstForm) {
    body.spec = definitionSpec(firstForm);
  }
  const testValue = testInput?.value.trim();
  if (testValue) body.test_value = Number(testValue);
  return body;
}

function initializePreview(panel: HTMLElement): void {
  const signalRef = panel.dataset.signalRef;
  if (!signalRef) return;
  const previewScope =
    panel.closest<HTMLElement>(".sensor-setting-workspace") ?? document.body;
  const forms = queryAll<HTMLFormElement>(
    `form.semantic-form[data-signal-ref="${signalRef}"]`,
  );
  const calibrationForm = query<HTMLFormElement>(
    `form[action="/console/signals/${signalRef}/calibration"]`,
  );
  const multipleRules =
    forms.some((form) => !!form.dataset.ruleId) ||
    forms.some((form) => form.action.endsWith("/semantic-rules"));
  const testInput = query<HTMLInputElement>(
    '[name="preview_test_value"]',
    panel,
  );
  const testResult = query<HTMLElement>("[data-preview-test-result]", panel);
  const range = query<HTMLElement>("[data-preview-range]", panel);
  const count = query<HTMLElement>("[data-preview-count]", panel);
  const message = query<HTMLElement>("[data-preview-message]", panel);
  const feedState = query<HTMLElement>("[data-preview-feed-state]", panel);
  const checkedAt = query<HTMLElement>("[data-preview-checked-at]", panel);
  const accessibleSummary = query<HTMLElement>(
    "[data-preview-accessible-summary]",
    panel,
  );
  const toggle = query<HTMLButtonElement>("[data-preview-toggle]", panel);
  const chart = query<SVGSVGElement>("[data-preview-chart]", panel);
  const currentValue = query<HTMLElement>(
    "[data-preview-current-value]",
    panel,
  );
  const currentReceived = query<HTMLElement>(
    "[data-preview-current-received]",
    panel,
  );
  const unit = panel.dataset.unit ?? "";
  if (!range || !count || !message || !chart) return;

  const sourceSummary = query<HTMLElement>(
    ".sensor-detail-latest[data-source-value]",
  );
  const sourceCurrentValue = sourceSummary
    ? query<HTMLElement>("[data-source-current-value]", sourceSummary)
    : null;
  const sourceCurrentReceived = sourceSummary
    ? query<HTMLElement>("[data-source-current-received]", sourceSummary)
    : null;
  const valueKind = query<HTMLSelectElement>(
    'form[data-signal-profile] [name="display_value_kind"]',
  );
  const decimalPlaces = query<HTMLInputElement>(
    'form[data-signal-profile] [name="decimal_places"]',
  );
  let controller: AbortController | undefined;
  let debounce: number | undefined;
  let previewUnavailable = false;
  let paused = false;
  let lastSeenReceivedAt: number | undefined;

  const setFeedState = (state: string): void => {
    if (feedState) setText(feedState, state);
  };

  const markChecked = (): void => {
    if (!checkedAt) return;
    setText(
      checkedAt,
      `確認 ${new Date().toLocaleTimeString("ja-JP", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      })}`,
    );
  };

  const refresh = async (): Promise<void> => {
    controller?.abort();
    controller = new AbortController();
    clearFieldErrors(previewScope);
    const activeID = activePreviewID(previewScope);

    const body = buildRequest(
      signalRef,
      forms,
      calibrationForm,
      multipleRules,
      testInput,
      activeID,
    );
    try {
      const result = await createMappingPreview(
        body,
        csrfToken(),
        controller.signal,
      );
      if (!result.ok) {
        const fieldName = result.error?.error.field;
        const activeForm = forms.find(
          (candidate) => candidate.dataset.previewId === activeID,
        );
        const invalidField =
          fieldName && activeForm
            ? formField(activeForm, fieldName)
            : null;
        const fieldLabel = invalidField
          ?.closest("label")
          ?.querySelector(":scope > span")
          ?.textContent?.trim();
        if (result.status === 404 && !forms[0]) {
          previewUnavailable = true;
          renderRuleResult(panel, null, "none", unit);
          setFeedState("表示するルールがありません");
          setText(
            message,
            "値の変換が設定されると、ここに設定結果を表示します。",
          );
        } else if (fieldLabel && invalidField) {
          renderRuleResult(panel, null, "invalid", unit);
          setFeedState("設定内容を確認してください");
          showFieldError(invalidField, fieldLabel);
          setText(message,
            `${fieldLabel}を確認してください。` +
            "最後に確認できたグラフを表示しています。",
          );
        } else {
          renderRuleResult(
            panel,
            null,
            result.status === 400 ? "invalid" : "error",
            unit,
          );
          setFeedState("更新を確認できません");
          setText(
            message,
            "設定内容を確認してください。最後に確認できたグラフを表示しています。",
          );
        }
        return;
      }

      const selection = selectPreview(result.value, activeID);
      const selectedReady =
        selection.selected && !selection.selected.error
          ? selection.selected
          : null;
      const payload =
        selectedReady ??
        (selection.raw ? rawOnlyPreview(selection.raw) : null);
      if (!payload) {
        renderRuleResult(
          panel,
          null,
          activeID ? "error" : "none",
          unit,
        );
        setFeedState("表示するルールがありません");
        setText(message, "確認できるルールがありません。");
        return;
      }
      renderPreviewChart(chart, payload);
      const resultState: "ready" | "none" | "error" = !activeID
        ? "none"
        : selectedReady
          ? "ready"
          : "error";
      const outcome = renderRuleResult(
        panel,
        selectedReady,
        resultState,
        unit,
      );
      updateAccessibleSummary(
        accessibleSummary,
        payload,
        selectedReady,
        outcome,
      );
      const points = payload.points ?? [];
      const latest = selection.raw?.points?.at(-1);
      markChecked();
      if (!latest) {
        setFeedState("受信待ち");
      } else if (lastSeenReceivedAt === undefined) {
        setFeedState("実データを表示中");
        lastSeenReceivedAt = latest.received_at;
      } else if (latest.received_at > lastSeenReceivedAt) {
        setFeedState("新しいデータを受信");
        lastSeenReceivedAt = latest.received_at;
      } else {
        setFeedState("新着なし");
      }
      if (latest && currentValue) {
        currentValue.textContent = formatCurrentValue(
          latest.input,
          valueKind?.value,
          decimalPlaces ? Number(decimalPlaces.value) : undefined,
        );
      }
      if (latest && sourceCurrentValue && sourceSummary) {
        const rawValue = formatCurrentValue(
          latest.input,
          valueKind?.value,
          decimalPlaces ? Number(decimalPlaces.value) : undefined,
        );
        sourceCurrentValue.textContent = rawValue;
        sourceSummary.dataset.sourceValue = rawValue;
      }
      if (latest && (currentReceived || sourceCurrentReceived)) {
        const elapsed = Math.max(0, Date.now() - latest.received_at);
        const relative =
          elapsed < 5_000
            ? "たった今"
            : elapsed < 60_000
              ? `${Math.floor(elapsed / 1_000)}秒前`
              : `${Math.floor(elapsed / 60_000)}分前`;
        const receivedTitle = new Date(latest.received_at).toLocaleString(
          "ja-JP",
        );
        if (currentReceived) {
          currentReceived.textContent = `最終受信 ${relative}`;
          currentReceived.title = receivedTitle;
        }
        if (sourceCurrentReceived) {
          sourceCurrentReceived.textContent = relative;
          sourceCurrentReceived.title = receivedTitle;
        }
      }

      if (payload.input_count === 0) {
        range.textContent = "受信データはまだありません";
        count.textContent = "試す値で設定結果を確認できます。";
        setText(message, "履歴は作らず、実際に届いた値だけを表示します。");
      } else {
        const window = previewWindow(payload, points);
        range.textContent = `直近${formatDuration(window.start, window.end)}の受信値`;
        count.textContent =
          `${payload.input_count.toLocaleString("ja-JP")}件を` +
          `${payload.plot_count.toLocaleString("ja-JP")}点で表示`;
        const valuesOverlap = points.every(
          (point) => Math.abs(point.input - point.calibrated) < 1e-9,
        );
        setText(
          message,
          payload.truncated_by === "input_count"
            ? "高速な信号のため、最新20,000件を要約しています。"
            : payload.kind === "cumulative_counter"
              ? `表示範囲内の累積値は ${points.at(-1)?.counter ?? 0} です。` +
                "先頭の値は数えません。"
              : valuesOverlap
                ? "変換前後の値は同じです。補正を変更すると差を確認できます。"
              : "設定を変えると、保存前の結果をこのグラフで確認できます。",
        );
      }

      if (testResult) {
        const previewResult = payload.test_result;
        if (!previewResult) {
          testResult.textContent = "値を入力すると結果を確認できます";
        } else {
          switch (payload.kind) {
            case "boolean":
              testResult.textContent = previewResult.boolean ? "ON" : "OFF";
              break;
            case "alarm":
              testResult.textContent = previewResult.boolean ? "異常" : "正常";
              break;
            case "cumulative_counter":
              testResult.textContent =
                previewResult.integer !== undefined
                  ? `累積 ${formatNumber(previewResult.integer)}`
                  : "最初の値として確認（累積には加えません）";
              break;
            default:
              testResult.textContent =
                previewResult.number !== undefined
                  ? formatNumber(previewResult.number)
                  : `補正後 ${formatNumber(previewResult.calibrated)}`;
              break;
          }
        }
      }
    } catch (error: unknown) {
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        renderRuleResult(panel, null, "error", unit);
        setFeedState("更新を確認できません");
        setText(
          message,
          "設定結果を更新できません。データ受信には影響ありません。",
        );
      }
    }
  };

  const schedule = (): void => {
    if (debounce !== undefined) window.clearTimeout(debounce);
    debounce = window.setTimeout(refresh, 300);
  };
  for (const form of forms) {
    form.addEventListener("input", schedule);
    form.addEventListener("change", schedule);
  }
  calibrationForm?.addEventListener("input", schedule);
  calibrationForm?.addEventListener("change", schedule);
  previewScope.addEventListener(SETTING_TAB_CHANGE_EVENT, schedule);
  for (const target of queryAll<HTMLDetailsElement>(
    "details[data-preview-target]",
    previewScope,
  )) {
    target.addEventListener("toggle", schedule);
  }
  testInput?.addEventListener("input", schedule);
  toggle?.addEventListener("click", () => {
    paused = !paused;
    toggle.setAttribute("aria-checked", String(!paused));
    const state = query<HTMLElement>("[data-preview-toggle-state]", toggle);
    if (state) state.textContent = paused ? "OFF" : "ON";
    panel.classList.toggle("preview-paused", paused);
    if (paused) {
      setFeedState("更新停止中");
    } else {
      setFeedState("受信データを確認中");
      void refresh();
    }
  });
  void refresh();
  window.setInterval(() => {
    if (
      document.visibilityState === "visible" &&
      !previewUnavailable &&
      !paused
    ) {
      void refresh();
    }
  }, 1000);
}

export function initializePreviews(): void {
  for (const panel of queryAll<HTMLElement>("[data-setting-simulation]")) {
    initializePreview(panel);
  }
}
