import type { components } from "./generated/edge-api";
import {
  createMappingPreview,
  getHistorySeries,
  type HistorySeries,
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
import { renderSignalChart } from "./signal-chart";

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

type CounterHistory =
  | { status: "pending" }
  | { status: "available"; value: HistorySeries }
  | { status: "unavailable" };

interface CounterPreviewState {
  persisted: boolean;
  history?: CounterHistory;
  session?: CounterHistorySession;
}

interface CounterHistorySessionPoint {
  at: number;
  value: number;
  minimum: number;
  maximum: number;
  sampleCount: number;
}

interface CounterHistorySession {
  startedAt: number;
  baselineCaptured: boolean;
  points: CounterHistorySessionPoint[];
}

const COUNTER_WINDOW_MS = 60_000;
const COUNTER_BUCKET_MS = 1_000;
const COUNTER_SESSION_MAX_POINTS = 60;

const kindLabels: Record<SemanticKind, string> = {
  numeric: "測定値",
  boolean: "ON / OFF",
  cumulative_counter: "累積値",
  alarm: "異常検知",
};

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

function pointPlotAt(point: PreviewPoint): number {
  return isFiniteNumber(point.plot_at) ? Number(point.plot_at) : point.received_at;
}

function latestPreviewPoint(payload: PreviewBody): PreviewPoint | undefined {
  return payload.latest_point ?? payload.points?.at(-1) ?? undefined;
}

function hasMeaningfulResult(points: PreviewPoint[]): boolean {
  const epsilon = 1e-9;
  return points.some(
    (point) =>
      Math.abs(point.calibrated - point.input) > epsilon ||
      Math.abs(point.calibrated_min - point.input_min) > epsilon ||
      Math.abs(point.calibrated_max - point.input_max) > epsilon,
  );
}

function counterWindowDelta(payload: PreviewBody): number {
  const points = payload.points ?? [];
  const window = previewWindow(payload, points);
  return points.reduce((total, point) => {
    const at = pointPlotAt(point);
    if (at < window.start || at > window.end) return total;
    return total + Math.max(0, Number(point.increment ?? 0));
  }, 0);
}

function persistedCounterRuleID(
  forms: HTMLFormElement[],
  activeID: string | undefined,
  selected: SemanticRulePreview | null,
): string | undefined {
  if (!selected || selected.kind !== "cumulative_counter" || !activeID) {
    return undefined;
  }
  return counterRuleIDForActiveForm(forms, activeID);
}

function counterRuleIDForActiveForm(
  forms: HTMLFormElement[],
  activeID: string | undefined,
): string | undefined {
  const form = forms.find((candidate) => candidate.dataset.previewId === activeID);
  return form?.dataset.ruleId && formField(form, "kind")?.value === "cumulative_counter"
    ? form.dataset.ruleId
    : undefined;
}

async function loadCounterHistory(
  ruleID: string,
  signal: AbortSignal,
): Promise<CounterHistory> {
  const end = Date.now();
  try {
    const result = await getHistorySeries(
      ruleID,
      end - COUNTER_WINDOW_MS,
      end,
      COUNTER_BUCKET_MS,
      signal,
    );
    return result.ok
      ? { status: "available", value: result.value }
      : { status: "unavailable" };
  } catch (error: unknown) {
    if (error instanceof DOMException && error.name === "AbortError") {
      throw error;
    }
    return { status: "unavailable" };
  }
}

function availableCounterHistory(
  state: CounterPreviewState,
): HistorySeries | undefined {
  return state.history?.status === "available" ? state.history.value : undefined;
}

function latestHistoryValue(history: HistorySeries | undefined): number | undefined {
  return history &&
    typeof history.latest_value === "number" &&
    Number.isFinite(history.latest_value)
    ? Number(history.latest_value)
    : undefined;
}

function latestHistoryReceivedAt(history: HistorySeries): number | undefined {
  return typeof history.latest_received_at === "number" &&
    Number.isFinite(history.latest_received_at)
    ? history.latest_received_at
    : undefined;
}

function retainLatestHistory(
  previous: HistorySeries | undefined,
  next: HistorySeries,
): HistorySeries {
  if (
    latestHistoryValue(next) !== undefined ||
    previous === undefined ||
    latestHistoryValue(previous) === undefined
  ) {
    return next;
  }
  return {
    ...next,
    latest_received_at: previous.latest_received_at,
    latest_value: previous.latest_value,
  };
}

function appendCounterSessionPoint(
  points: CounterHistorySessionPoint[],
  point: CounterHistorySessionPoint,
): CounterHistorySessionPoint[] {
  const previous = points.at(-1);
  if (previous === undefined) return [point];
  if (point.at < previous.at) return points;
  if (point.at === previous.at) {
    const replaced = [...points.slice(0, -1), point];
    return replaced.length > 1 && replaced.at(-2)?.value === point.value
      ? replaced.slice(0, -1)
      : replaced;
  }
  if (point.value === previous.value) return points;
  return [...points, point].slice(-COUNTER_SESSION_MAX_POINTS);
}

function counterCurrentPointAt(
  session: CounterHistorySession,
  latestReceivedAt: number | undefined,
  capturedAt: number,
): number {
  const previousAt = session.points.at(-1)?.at;
  const minimumAt = previousAt === undefined
    ? session.startedAt
    : previousAt + 1;
  if (
    latestReceivedAt !== undefined && latestReceivedAt >= minimumAt
  ) {
    return latestReceivedAt;
  }
  return Math.max(
    Number.isFinite(capturedAt) ? capturedAt : minimumAt,
    minimumAt,
  );
}

function mergeCounterHistorySession(
  session: CounterHistorySession,
  history: HistorySeries,
  capturedAt: number,
): CounterHistorySession {
  let baselineCaptured = session.baselineCaptured;
  let points = session.points;
  const latestValue = latestHistoryValue(history);
  const latestReceivedAt = latestHistoryReceivedAt(history);
  if (!baselineCaptured) {
    baselineCaptured = true;
    if (latestValue !== undefined) {
      const at = latestReceivedAt !== undefined &&
          latestReceivedAt >= session.startedAt
        ? counterCurrentPointAt(session, latestReceivedAt, capturedAt)
        : session.startedAt;
      points = appendCounterSessionPoint(points, {
        at,
        value: latestValue,
        minimum: latestValue,
        maximum: latestValue,
        sampleCount: 1,
      });
    }
  }
  if (
    baselineCaptured &&
    latestValue !== undefined &&
    points.at(-1)?.value !== latestValue
  ) {
    const at = counterCurrentPointAt(
      { ...session, points },
      latestReceivedAt,
      capturedAt,
    );
    points = appendCounterSessionPoint(points, {
      at,
      value: latestValue,
      minimum: latestValue,
      maximum: latestValue,
      sampleCount: 1,
    });
  }
  return { ...session, baselineCaptured, points };
}

function renderCounterHistoryChart(
  svg: SVGSVGElement,
  state: CounterPreviewState,
): number {
  if (state.history?.status !== "available") {
    const unavailable = state.history?.status === "unavailable";
    return renderSignalChart(svg, {
      points: [],
      geometry: "compact",
      axisLabels: { start: "表示開始", end: "現在" },
      emptyTitle: unavailable
        ? "表示開始後の保存済み累積履歴を取得できません"
        : "表示開始後の保存済み累積履歴を読み込んでいます",
      emptyHint: unavailable
        ? "接続を確認して、もう一度表示してください"
        : "保存済みの結果を確認しています",
      title: "横軸は表示開始後の時間、縦軸は保存済み累積値です。表示開始後の最新最大60点を示し、61点目から最古点を外します。",
    });
  }
  const chartPoints = state.session?.points ?? [];
  const startAt = chartPoints[0]?.at ?? state.session?.startedAt;
  const endAt = chartPoints.at(-1)?.at ?? startAt;
  return renderSignalChart(svg, {
    points: chartPoints,
    geometry: "compact",
    rawStep: true,
    ...(startAt === undefined ? {} : { startAt }),
    ...(endAt === undefined ? {} : { endAt }),
    showLatestMarker: chartPoints.length > 0,
    ...(endAt === undefined ? {} : { latestAt: endAt }),
    emptyTitle: "表示開始後の保存済み累積変化はありません",
    emptyHint: "保存済みの意味結果が変化すると、表示開始後の最新最大60点を表示します",
    title: "横軸は表示開始後の時間、縦軸は保存済み累積値です。表示開始後の最新最大60点を示し、61点目から最古点を外します。",
  });
}

function counterSummaryText(
  state: CounterPreviewState,
  plottedCount: number,
): string {
  if (state.history?.status === "pending") {
    return "表示開始後の保存済み累積履歴を読み込んでいます。";
  }
  if (state.history?.status === "unavailable") {
    return "表示開始後の保存済み累積履歴を取得できません。";
  }
  const history = availableCounterHistory(state);
  const latestValue = latestHistoryValue(history);
  return latestValue === undefined
    ? "表示開始後の保存済み累積変化はありません。"
    : `${formatNumber(latestValue)}（保存済み、表示開始後の${plottedCount}点／最新最大60点）`;
}

function counterPreviewMessage(
  state: CounterPreviewState,
): string {
  if (!state.persisted) return "保存後に累積開始。";
  if (state.history?.status === "pending") {
    return "表示開始後の保存済み累積値を読み込んでいます。";
  }
  if (state.history?.status === "unavailable") {
    return "表示開始後の保存済み累積履歴を取得できません。";
  }
  return latestHistoryValue(availableCounterHistory(state)) === undefined
    ? "表示開始後の保存済み累積変化はありません。"
    : "保存済み累積値は表示開始後の変化グラフで確認できます。";
}

function counterHistorySignature(state: CounterPreviewState): string {
  if (!state.history) return "none";
  if (state.history.status !== "available") return state.history.status;
  return JSON.stringify(state.session?.points ?? []);
}

function latestRuleOutcome(
  payload: PreviewBody,
  unit: string,
  counterState: CounterPreviewState = { persisted: false },
): RuleOutcome {
  const latest = latestPreviewPoint(payload);
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
      {
        const delta = counterWindowDelta(payload);
        const history = availableCounterHistory(counterState);
        const persistedTotal = latestHistoryValue(history);
        if (counterState.persisted) {
          if (counterState.history?.status === "pending") {
            return {
              value: "累積値を読み込み中",
              detail: `保存済み累積値を確認しています。この設定なら直近60秒で +${formatNumber(delta)}`,
              alarm: false,
            };
          }
          if (counterState.history?.status === "unavailable") {
            return {
              value: "保存済み累積値を取得できません",
              detail: `保存済み累積履歴を取得できません。この設定なら直近60秒で +${formatNumber(delta)}`,
              alarm: false,
            };
          }
          if (persistedTotal === undefined) {
            return {
              value: "表示開始後の保存済み累積変化はありません",
              detail: `保存済みの意味結果が届くと累積値を表示します。この設定なら直近60秒で +${formatNumber(delta)}`,
              alarm: false,
            };
          }
          return {
            value: `累積 ${formatNumber(persistedTotal)}`,
            detail: `この設定なら直近60秒で +${formatNumber(delta)}`,
            alarm: false,
          };
        }
        return {
          value: `直近60秒で +${formatNumber(delta)}`,
          detail: "保存後に累積開始。保存済み累積値はここに表示されます。",
          alarm: false,
        };
      }
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
  counterState?: CounterPreviewState,
): RuleOutcome | null {
  const container = query<HTMLElement>("[data-preview-rule-result]", panel);
  const name = query<HTMLElement>("[data-preview-rule-name]", panel);
  const kind = query<HTMLElement>("[data-preview-rule-kind]", panel);
  const value = query<HTMLElement>("[data-preview-rule-value]", panel);
  const detail = query<HTMLElement>("[data-preview-rule-detail]", panel);
  if (!container || !name || !kind || !value || !detail) return null;

  container.classList.remove("is-alarm");
  if (state === "error" && selected?.error) {
    setText(
      name,
      `${selected.display_name}（判定結果を更新できません）`,
    );
    setText(kind, kindLabel(selected.kind));
    setText(value, "—");
    setText(detail, "受信値はそのまま確認できます。");
    return null;
  }
  if (state !== "ready" || !selected) {
    const messages = {
      none: [
        "選択中のルールはありません",
        "—",
        "保存済みルールを選択すると判定結果を確認できます。",
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

  const outcome = latestRuleOutcome(selected, unit, counterState);
  setText(name, selected.display_name);
  setText(kind, kindLabel(selected.kind));
  setText(value, outcome.value);
  setText(detail, outcome.detail);
  container.classList.toggle("is-alarm", outcome.alarm);
  return outcome;
}

function clearAuxiliaryOutputs(
  summary: HTMLElement | null,
  state: "none" | "invalid" | "error",
): void {
  const messages = {
    none: "グラフに表示できる受信データはまだありません。",
    invalid: "設定内容を確認してください。受信値はそのまま確認できます。",
    error: "判定結果を更新できません。受信値はそのまま確認できます。",
  } as const;
  if (summary) setText(summary, messages[state]);
}

function updateAccessibleSummary(
  summary: HTMLElement | null,
  raw: PreviewBody,
  selected: SemanticRulePreview | null,
  outcome: RuleOutcome | null,
  unit: string,
  plotPoints: PreviewPoint[] = raw.points ?? [],
): void {
  if (!summary) return;
  const points = plotPoints;
  if (selected?.error) {
    if (!points.length) {
      setText(
        summary,
        `受信値はまだありません。選択中は${selected.display_name}、` +
          `${kindLabel(selected.kind)}ですが、判定結果を更新できません。`,
      );
      return;
    }
    const inputs = points.flatMap((point) => [
      Number(point.input_min),
      Number(point.input_max),
    ]);
    const evaluatedCount = raw.input_count ?? points.length;
    const bucketCount = points.length;
    setText(
      summary,
      `受信値は${formatNumber(Math.min(...inputs))}から` +
        `${formatNumber(Math.max(...inputs))}です。` +
        `選択中は${selected.display_name}、${kindLabel(selected.kind)}ですが、` +
        "判定結果を更新できません。受信値はそのまま確認できます。" +
        `${evaluatedCount}件を評価し、直近60秒の${bucketCount}bucketを表示しています。`,
    );
    return;
  }
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
  const evaluatedCount = raw.input_count ?? points.length;
  const bucketCount = points.length;
  const ruleText = selected && outcome
    ? selected.kind === "cumulative_counter"
      ? `選択中は${selected.display_name}、${kindLabel(selected.kind)}、` +
        `${outcome.value}です。${outcome.detail}`
      : `選択中は${selected.display_name}、${kindLabel(selected.kind)}、現在は${outcome.value}です。`
    : "選択中のルールはありません。";
  const calibratedText =
    selected?.kind === "numeric" && outcome
      ? (() => {
          const calibrated = points.flatMap((point) => [
            Number(point.calibrated_min),
            Number(point.calibrated_max),
          ]);
          const latest = latestPreviewPoint(selected);
          if (!latest || !calibrated.length) return "";
          return (
            `補正後は${formatNumber(Math.min(...calibrated))}から` +
            `${formatNumber(Math.max(...calibrated))}、最新の補正後は` +
            `${formatNumber(latest.calibrated)}${unit ? ` ${unit}` : ""}です。`
          );
        })()
      : "";
  setText(
    summary,
    `受信値は${formatNumber(Math.min(...inputs))}から` +
      `${formatNumber(Math.max(...inputs))}です。${calibratedText}${ruleText}` +
      `${evaluatedCount}件を評価し、直近60秒の${bucketCount}bucketを表示しています。`,
  );
}

function previewWindow(
  payload: PreviewBody,
  points: PreviewPoint[],
): { start: number; end: number } {
  const now = Date.now();
  const end = payload.window_end ?? (
    points.at(-1) ? pointPlotAt(points.at(-1)!) : now
  );
  return {
    start: payload.window_start ?? (points[0] ? pointPlotAt(points[0]) : end),
    end,
  };
}

function renderPreviewChart(
  svg: SVGSVGElement,
  payload: PreviewBody,
  showSemanticOverlays: boolean,
  unit: string,
  rawBoolean: boolean,
  showResult: boolean,
): PreviewPoint[] {
  const points = payload.points ?? [];
  const window = previewWindow(payload, points);
  renderSignalChart(svg, {
    points: points.map((point) => ({
      at: pointPlotAt(point),
      value: point.input,
      minimum: point.input_min,
      maximum: point.input_max,
      sampleCount: point.sample_count,
      result: point.calibrated,
      resultMinimum: point.calibrated_min,
      resultMaximum: point.calibrated_max,
      activeRatio: point.sample_count
        ? Number(point.active_samples ?? 0) / point.sample_count
        : 0,
    })),
    geometry: "compact",
    unit,
    boolean: rawBoolean && !showSemanticOverlays,
    rawStep: rawBoolean,
    startAt: window.start,
    endAt: window.end,
    showResult,
    resultStep: showResult && rawBoolean,
    showLatestMarker: showSemanticOverlays,
    showActiveBands:
      showSemanticOverlays &&
      payload.kind !== "numeric" &&
      payload.kind !== "cumulative_counter",
    thresholds: showSemanticOverlays
      ? { rise: payload.rise_threshold, fall: payload.fall_threshold }
      : undefined,
    emptyTitle: payload.error
      ? "このルールでは受信値を判定できません"
      : "まだ受信データがありません",
    emptyHint: payload.error
      ? "入力値の補正と判定条件を確認してください"
      : "実際に届いた値を待っています",
    title: `横軸は直近60秒、縦軸は受信値${unit ? `（${unit}）` : ""}${showResult ? "と設定結果" : ""}です。`,
  });
  return points;
}

function isMultipleRulePreview(
  response: MappingPreviewResponse,
): response is components["schemas"]["MultipleRuleMappingPreview"] {
  return "rules" in response;
}

function activeSettingPanel(scope: HTMLElement): HTMLElement | undefined {
  return queryAll<HTMLElement>(
    "[data-setting-panel]",
    scope,
  ).find((panel) => !panel.hidden);
}

function previewTargetID(target: HTMLDetailsElement): string | undefined {
  return target.dataset.ruleId ||
    query<HTMLFormElement>("form.semantic-form[data-preview-id]", target)
      ?.dataset.previewId;
}

function previewTargets(panel: HTMLElement): HTMLDetailsElement[] {
  return queryAll<HTMLDetailsElement>(
    "details[data-preview-target]",
    panel,
  ).filter((target) => !!previewTargetID(target));
}

function persistedPreviewTargets(panel: HTMLElement): HTMLDetailsElement[] {
  return previewTargets(panel).filter((target) => !!target.dataset.ruleId);
}

function previewTargetLabel(target: HTMLDetailsElement): string {
  const form = query<HTMLFormElement>("form.semantic-form", target);
  const name = form ? formField(form, "display_name")?.value.trim() : undefined;
  const targetID = previewTargetID(target);
  return name ||
    (targetID === "draft-alarm"
      ? "新しい異常検知"
      : targetID === "draft-normal"
        ? "新しい計測ルール"
        : undefined) ||
    query<HTMLElement>("summary strong", target)?.textContent?.trim() ||
    targetID ||
    "ルール";
}

function renderPreviewRuleOptions(
  selector: HTMLSelectElement | null,
  targets: HTMLDetailsElement[],
  selectedID: string | undefined,
): void {
  if (!selector) return;
  selector.replaceChildren();
  if (!targets.length) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "選択できるルールなし";
    option.disabled = true;
    option.selected = true;
    selector.append(option);
    selector.disabled = true;
    return;
  }
  const hasPersistedTarget = targets.some((target) => !!target.dataset.ruleId);
  if (!hasPersistedTarget) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "ルールを選択";
    option.disabled = true;
    option.selected = !selectedID;
    selector.append(option);
  }
  selector.disabled = false;
  for (const target of targets) {
    const option = document.createElement("option");
    option.value = previewTargetID(target) ?? "";
    option.textContent = previewTargetLabel(target);
    selector.append(option);
  }
  const selectedTarget = targets.find(
    (target) => previewTargetID(target) === selectedID,
  );
  const firstPersistedTarget = targets.find((target) => !!target.dataset.ruleId);
  selector.value = selectedTarget
    ? previewTargetID(selectedTarget) ?? ""
    : firstPersistedTarget
      ? previewTargetID(firstPersistedTarget) ?? ""
      : "";
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
  const rawPoint = (point: PreviewPoint): PreviewPoint => ({
    ...point,
    calibrated: point.input,
    calibrated_min: point.input_min,
    calibrated_max: point.input_max,
    active: undefined,
    active_samples: undefined,
    transitions: undefined,
    counter: undefined,
    increment: undefined,
  });
  return {
    ...payload,
    kind: "numeric",
    rise_threshold: undefined,
    fall_threshold: undefined,
    points: (payload.points ?? []).map(rawPoint),
    latest_point: payload.latest_point
      ? rawPoint(payload.latest_point)
      : undefined,
  };
}

function buildRequest(
  signalRef: string,
  forms: HTMLFormElement[],
  calibrationForm: HTMLFormElement | null,
  multipleRules: boolean,
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
  const rawBoolean = forms.some(
    (form) => form.dataset.booleanInput === "true",
  );
  const range = query<HTMLElement>("[data-preview-range]", panel);
  const count = query<HTMLElement>("[data-preview-count]", panel);
  const message = query<HTMLElement>("[data-preview-message]", panel);
  const feedState = query<HTMLElement>("[data-preview-feed-state]", panel);
  const checkedAt = query<HTMLElement>("[data-preview-checked-at]", panel);
  const toggle = query<HTMLButtonElement>("[data-preview-toggle]", panel);
  const chart = query<SVGSVGElement>("[data-preview-chart]", panel);
  const accessibleSummaryID = chart?.getAttribute("aria-describedby");
  const accessibleSummary = accessibleSummaryID
    ? document.getElementById(accessibleSummaryID)
    : null;
  const counterPanel = query<HTMLElement>("[data-preview-counter]", panel);
  const counterChart = query<SVGSVGElement>(
    "[data-preview-counter-chart]",
    panel,
  );
  const counterSummary = query<HTMLElement>(
    "[data-preview-counter-summary]",
    panel,
  );
  const resultLegend = query<HTMLElement>(
    "[data-preview-result-legend]",
    panel,
  );
  const thresholdLegend = query<HTMLElement>(
    "[data-preview-threshold-legend]",
    panel,
  );
  const currentValue = query<HTMLElement>(
    "[data-preview-current-value]",
    panel,
  );
  const currentReceived = query<HTMLElement>(
    "[data-preview-current-received]",
    panel,
  );
  const ruleSelector = query<HTMLSelectElement>(
    "[data-preview-rule-select]",
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
  let counterHistoryController: AbortController | undefined;
  let counterHistoryRuleID: string | undefined;
  let counterHistory: CounterHistory | undefined;
  let counterHistorySession: CounterHistorySession | undefined;
  let lastAvailableCounterHistory: HistorySeries | undefined;
  let renderedCounterHistoryKey: string | undefined;
  let renderedCounterHistoryPointCount = 0;
  let renderCurrentCounter:
    | ((state: CounterPreviewState) => void)
    | undefined;
  let debounce: number | undefined;
  let previewUnavailable = false;
  let paused = false;
  let lastSeenReceivedAt: number | undefined;
  let selectedPreviewID: string | undefined;
  const lastSelectedTargetByPanel = new WeakMap<HTMLElement, string>();
  const pendingInitialToggleStates = new Map<HTMLDetailsElement, boolean>();

  const setCounterPanel = (visible: boolean): void => {
    if (counterPanel) counterPanel.hidden = !visible;
  };

  const setSemanticLegends = (
    semanticVisible: boolean,
    resultVisible: boolean,
    payload?: PreviewBody | null,
  ): void => {
    if (resultLegend) resultLegend.hidden = !resultVisible;
    if (thresholdLegend) {
      thresholdLegend.hidden =
        !semanticVisible ||
        !payload ||
        (!isFiniteNumber(payload.rise_threshold) &&
          !isFiniteNumber(payload.fall_threshold));
    }
  };

  const hideSemanticAuxiliaries = (): void => {
    setSemanticLegends(false, false);
    setCounterPanel(false);
    renderedCounterHistoryKey = undefined;
    renderedCounterHistoryPointCount = 0;
  };

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

  const selectPreviewTarget = (
    selectedTarget: HTMLDetailsElement | undefined,
  ): void => {
    const activePanel = activeSettingPanel(previewScope);
    const targets = activePanel ? previewTargets(activePanel) : [];
    const selectedID = selectedTarget ? previewTargetID(selectedTarget) : undefined;
    if (!activePanel || !selectedTarget || !selectedID || !targets.includes(selectedTarget)) {
      selectedPreviewID = undefined;
      if (activePanel) lastSelectedTargetByPanel.delete(activePanel);
      renderPreviewRuleOptions(ruleSelector, targets, undefined);
      return;
    }
    selectedPreviewID = selectedID;
    lastSelectedTargetByPanel.set(activePanel, selectedID);
    renderPreviewRuleOptions(ruleSelector, targets, selectedID);
    for (const target of queryAll<HTMLDetailsElement>(
      "details[data-preview-target]",
      activePanel,
    )) {
      target.open = target === selectedTarget;
    }
  };

  const restorePreviewTarget = (): void => {
    const activePanel = activeSettingPanel(previewScope);
    const targets = activePanel ? previewTargets(activePanel) : [];
    const persistedTargets = activePanel
      ? persistedPreviewTargets(activePanel)
      : [];
    const rememberedID = activePanel
      ? lastSelectedTargetByPanel.get(activePanel)
      : undefined;
    selectPreviewTarget(
      targets.find((target) => previewTargetID(target) === rememberedID) ??
        persistedTargets[0],
    );
  };

  const refreshRuleSelectorLabels = (form: HTMLFormElement): void => {
    const activePanel = activeSettingPanel(previewScope);
    if (!activePanel || !activePanel.contains(form)) return;
    const targets = previewTargets(activePanel);
    const selectedID = targets.some(
      (target) => previewTargetID(target) === selectedPreviewID,
    )
      ? selectedPreviewID
      : undefined;
    renderPreviewRuleOptions(ruleSelector, targets, selectedID);
  };

  const counterStateFor = (
    ruleID: string | undefined,
  ): CounterPreviewState => {
    if (!ruleID) {
      counterHistoryController?.abort();
      counterHistoryController = undefined;
      counterHistoryRuleID = undefined;
      counterHistory = undefined;
      counterHistorySession = undefined;
      lastAvailableCounterHistory = undefined;
      renderedCounterHistoryKey = undefined;
      renderedCounterHistoryPointCount = 0;
      renderCurrentCounter = undefined;
      return { persisted: false };
    }
    if (counterHistoryRuleID !== ruleID) {
      counterHistoryController?.abort();
      counterHistoryController = undefined;
      counterHistoryRuleID = ruleID;
      counterHistory = { status: "pending" };
      counterHistorySession = {
        startedAt: Date.now(),
        baselineCaptured: false,
        points: [],
      };
      lastAvailableCounterHistory = undefined;
      renderedCounterHistoryKey = undefined;
      renderedCounterHistoryPointCount = 0;
    }
    return {
      persisted: true,
      history: counterHistory ?? { status: "pending" },
      session: counterHistorySession,
    };
  };

  const refreshCounterHistory = (ruleID: string): void => {
    if (counterHistoryController || counterHistoryRuleID !== ruleID) return;
    const historyController = new AbortController();
    counterHistoryController = historyController;
    void loadCounterHistory(ruleID, historyController.signal)
      .then((history) => {
        if (
          counterHistoryController !== historyController ||
          counterHistoryRuleID !== ruleID ||
          historyController.signal.aborted
        ) {
          return;
        }
        counterHistoryController = undefined;
        if (history.status === "available") {
          const value = retainLatestHistory(lastAvailableCounterHistory, history.value);
          lastAvailableCounterHistory = value;
          counterHistory = { status: "available", value };
          if (counterHistorySession) {
            counterHistorySession = mergeCounterHistorySession(
              counterHistorySession,
              history.value,
              Date.now(),
            );
          }
          renderCurrentCounter?.({
            persisted: true,
            history: counterHistory,
            session: counterHistorySession,
          });
          return;
        }
        counterHistory = history;
        renderCurrentCounter?.({
          persisted: true,
          history,
          session: counterHistorySession,
        });
      })
      .catch(() => {
        if (
          counterHistoryController !== historyController ||
          counterHistoryRuleID !== ruleID ||
          historyController.signal.aborted
        ) {
          return;
        }
        counterHistoryController = undefined;
        const unavailable: CounterHistory = { status: "unavailable" };
        counterHistory = unavailable;
        renderCurrentCounter?.({
          persisted: true,
          history: unavailable,
          session: counterHistorySession,
        });
      });
  };

  const refresh = async (): Promise<void> => {
    controller?.abort();
    renderCurrentCounter = undefined;
    const requestController = new AbortController();
    controller = requestController;
    clearFieldErrors(previewScope);
    const activeID = selectedPreviewID;
    const requestedCounterRuleID = counterRuleIDForActiveForm(forms, activeID);
    counterStateFor(requestedCounterRuleID);
    if (requestedCounterRuleID) {
      refreshCounterHistory(requestedCounterRuleID);
    }

    const body = buildRequest(
      signalRef,
      forms,
      calibrationForm,
      multipleRules,
      activeID,
    );
    try {
      const result = await createMappingPreview(
        body,
        csrfToken(),
        requestController.signal,
      );
      if (controller !== requestController || requestController.signal.aborted) return;
      if (!result.ok) {
        hideSemanticAuxiliaries();
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
          clearAuxiliaryOutputs(accessibleSummary, "none");
          setFeedState("表示するルールがありません");
          setText(
            message,
            "値の変換が設定されると、ここに設定結果を表示します。",
          );
        } else if (fieldLabel && invalidField) {
          renderRuleResult(panel, null, "invalid", unit);
          clearAuxiliaryOutputs(accessibleSummary, "invalid");
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
          clearAuxiliaryOutputs(
            accessibleSummary,
            result.status === 400 ? "invalid" : "error",
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
      const selectedFailure =
        selection.selected?.error ? selection.selected : null;
      const payload =
        selectedReady ??
        (selection.raw ? rawOnlyPreview(selection.raw) : null);
      if (!payload) {
        hideSemanticAuxiliaries();
        renderRuleResult(
          panel,
          null,
          activeID ? "error" : "none",
          unit,
        );
        clearAuxiliaryOutputs(
          accessibleSummary,
          activeID ? "error" : "none",
        );
        setFeedState("表示するルールがありません");
        setText(message, "確認できるルールがありません。");
        return;
      }
      const persistedRuleID = persistedCounterRuleID(
        forms,
        activeID,
        selectedReady,
      );
      const counterState = counterStateFor(persistedRuleID);
      const showResult =
        Boolean(selectedReady) && hasMeaningfulResult(payload.points ?? []);
      setSemanticLegends(Boolean(selectedReady), showResult, payload);
      if (!persistedRuleID) {
        setCounterPanel(false);
        renderedCounterHistoryKey = undefined;
        renderedCounterHistoryPointCount = 0;
      }
      const plottedPoints = renderPreviewChart(
        chart,
        payload,
        Boolean(selectedReady),
        unit,
        rawBoolean,
        showResult,
      );
      const points = plottedPoints;
      const renderPreviewMessage = (state: CounterPreviewState): void => {
        if (payload.input_count === 0) {
          range.textContent = "受信データはまだありません";
          count.textContent = "受信値が届くと設定結果を確認できます。";
          setText(message, "履歴は作らず、実際に届いた値だけを表示します。");
        } else {
          const window = previewWindow(payload, points);
          range.textContent = `直近${formatDuration(window.start, window.end)}の受信値`;
          count.textContent =
            `${payload.input_count.toLocaleString("ja-JP")}件を評価し、` +
            `直近60秒の${points.length.toLocaleString("ja-JP")}bucketを表示`;
          setText(
            message,
            payload.truncated_by === "input_count"
              ? "高速な信号のため、最新20,000件を要約しています。"
              : payload.kind === "cumulative_counter"
                ? `この設定なら直近60秒で +${formatNumber(counterWindowDelta(payload))}。` +
                  counterPreviewMessage(state)
                : !showResult
                  ? "変換前後の値は同じです。補正を変更すると差を確認できます。"
                  : "設定を変えると、保存前の結果をこのグラフで確認できます。",
          );
        }
        if (selectedFailure) {
          setText(
            message,
            "判定結果を更新できません。受信値はそのまま確認できます。",
          );
        }
      };
      const renderCounterState = (state: CounterPreviewState): void => {
        if (persistedRuleID && counterChart) {
          const historyKey = counterHistorySignature(state);
          let plottedCount = 0;
          if (historyKey !== renderedCounterHistoryKey) {
            plottedCount = renderCounterHistoryChart(
              counterChart,
              state,
            );
            renderedCounterHistoryKey = historyKey;
            renderedCounterHistoryPointCount = plottedCount;
          } else {
            plottedCount = renderedCounterHistoryPointCount;
          }
          setCounterPanel(true);
          if (counterSummary) {
            setText(counterSummary, counterSummaryText(state, plottedCount));
          }
        }
        const resultState: "ready" | "none" | "error" = !activeID
          ? "none"
          : selectedReady
            ? "ready"
            : "error";
        const outcome = renderRuleResult(
          panel,
          selectedReady ?? selectedFailure,
          resultState,
          unit,
          state,
        );
        updateAccessibleSummary(
          accessibleSummary,
          payload,
          selectedReady ?? selectedFailure,
          outcome,
          unit,
          plottedPoints,
        );
        renderPreviewMessage(state);
      };
      renderCurrentCounter = persistedRuleID ? renderCounterState : undefined;
      renderCounterState(counterState);
      if (persistedRuleID) {
        refreshCounterHistory(persistedRuleID);
      }
      const latest = selection.raw
        ? latestPreviewPoint(selection.raw)
        : undefined;
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

    } catch (error: unknown) {
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        hideSemanticAuxiliaries();
        renderRuleResult(panel, null, "error", unit);
        clearAuxiliaryOutputs(accessibleSummary, "error");
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
  restorePreviewTarget();
  for (const form of forms) {
    const onFormChange = (event: Event): void => {
      if (
        event.target instanceof HTMLInputElement &&
        event.target.name === "display_name"
      ) {
        refreshRuleSelectorLabels(form);
      }
      schedule();
    };
    form.addEventListener("input", onFormChange);
    form.addEventListener("change", onFormChange);
  }
  calibrationForm?.addEventListener("input", schedule);
  calibrationForm?.addEventListener("change", schedule);
  previewScope.addEventListener(SETTING_TAB_CHANGE_EVENT, () => {
    restorePreviewTarget();
    schedule();
  });
  ruleSelector?.addEventListener("change", () => {
    const activePanel = activeSettingPanel(previewScope);
    const selectedTarget = activePanel
      ? previewTargets(activePanel).find(
        (target) => previewTargetID(target) === ruleSelector.value,
      )
      : undefined;
    selectPreviewTarget(selectedTarget);
    schedule();
  });
  for (const target of queryAll<HTMLDetailsElement>(
    "details[data-preview-target]",
    previewScope,
  )) {
    pendingInitialToggleStates.set(target, target.open);
    target.addEventListener("toggle", () => {
      const initialOpen = pendingInitialToggleStates.get(target);
      if (initialOpen !== undefined) {
        pendingInitialToggleStates.delete(target);
        if (target.open === initialOpen) return;
      }
      const activePanel = activeSettingPanel(previewScope);
      if (
        target.open &&
        activePanel?.contains(target) &&
        previewTargets(activePanel).includes(target)
      ) {
        selectPreviewTarget(target);
      }
      schedule();
    });
  }
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
