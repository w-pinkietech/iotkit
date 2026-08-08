import { getHistorySeries, type HistorySeries } from "./api";
import { query, queryAll } from "./dom";
import { renderSignalChart, type SignalChartPoint } from "./signal-chart";

const REFRESH_MS = 5 * 1_000;
const BUCKET_MS = 1_000;
const MAX_HISTORY_BUCKETS = 1_000;
const MAX_NUMERIC_POINTS = MAX_HISTORY_BUCKETS;
const MAX_BOOLEAN_POINTS = MAX_HISTORY_BUCKETS;
const MAX_ACTIVE_CARDS = 12;

interface HistoryWindow {
  from: number;
  to: number;
  bucketMs: number;
}

function formatNumber(value: number, decimalPlaces = 1): string {
  return value.toLocaleString("ja-JP", {
    maximumFractionDigits: Math.max(0, decimalPlaces),
  });
}

function isBooleanKind(kind: string): boolean {
  return kind === "bool" || kind === "boolean" || kind === "alarm";
}

function isStepKind(kind: string): boolean {
  return isBooleanKind(kind) || kind === "cumulative_counter";
}

function relativeTime(receivedAt: number, now: number): string {
  const elapsed = Math.max(0, now - receivedAt);
  if (elapsed < 10_000) return "たった今";
  if (elapsed < 60_000) return `${Math.floor(elapsed / 1_000)}秒前`;
  if (elapsed < 60 * 60_000) {
    const minutes = Math.floor(elapsed / 60_000);
    const seconds = Math.floor((elapsed % 60_000) / 1_000);
    return `${minutes}分${seconds}秒前`;
  }
  return `${Math.floor(elapsed / (60 * 60_000))}時間前`;
}

function historyWindow(from: number, to: number): HistoryWindow {
  const duration = Math.max(BUCKET_MS, to - from);
  const bucketMs = Math.max(
    BUCKET_MS,
    Math.ceil(duration / MAX_HISTORY_BUCKETS / BUCKET_MS) * BUCKET_MS,
  );
  return { from, to, bucketMs };
}

function sessionPoints(
  payload: HistorySeries,
  boolean: boolean,
  sessionStartedAt: number,
): HistorySeries["points"] {
  const points = payload.points.filter(
    (point) => point.bucket_start >= sessionStartedAt,
  );
  if (!boolean) return points.slice(-MAX_NUMERIC_POINTS);
  const transitions: HistorySeries["points"] = [];
  for (const point of points) {
    const state = (point.last_value ?? point.average) >= 0.5 ? 1 : 0;
    if (transitions.at(-1)?.average === state) continue;
    transitions.push({
      ...point,
      minimum: state,
      average: state,
      maximum: state,
    });
  }
  return transitions.slice(-MAX_BOOLEAN_POINTS);
}

function renderChart(
  svg: SVGSVGElement,
  payload: HistorySeries,
  kind: string,
  unit: string,
  now: number,
  sessionStartedAt: number,
): number {
  const boolean = isBooleanKind(kind);
  const step = isStepKind(kind);
  const points = sessionPoints(payload, boolean, sessionStartedAt);
  const chartPoints: SignalChartPoint[] = points.map((point) => ({
    at: point.bucket_start,
    value:
      kind === "cumulative_counter"
        ? point.last_value ?? point.average
        : point.average,
    minimum: point.minimum,
    maximum: point.maximum,
    sampleCount: point.sample_count,
  }));
  return renderSignalChart(svg, {
    points: chartPoints,
    geometry: "compact",
    unit,
    boolean,
    rawStep: step,
    startAt: sessionStartedAt,
    endAt: Math.max(now, sessionStartedAt + 1_000),
    latestAt:
      payload.latest_received_at !== null &&
      payload.latest_received_at >= sessionStartedAt
        ? payload.latest_received_at
        : points.at(-1)?.bucket_start,
    showLatestMarker: true,
    axisLabels: {
      start: "開始",
      end: "現在",
    },
    emptyTitle: "この画面を開いてからの受信を待っています",
    emptyHint: "表示開始後の全期間を最大1,000bucketで表示します",
    title: boolean
      ? "横軸はこの画面を開いてからの全期間（最大1,000bucket）、縦軸は接点のON/OFFです。"
      : `横軸はこの画面を開いてからの全期間（最大1,000bucket）、縦軸は値${unit ? `（${unit}）` : ""}です。`,
  });
}

function setStatus(card: HTMLElement, label: string, className: string): void {
  const status = query<HTMLElement>("[data-live-status]", card);
  if (!status) return;
  status.textContent = label;
  status.className = `status-pill ${className}`;
}

function retainCardLatest(
  previous: HistorySeries | undefined,
  payload: HistorySeries,
): HistorySeries {
  if (payload.latest_received_at !== null || previous?.latest_received_at === null || !previous) {
    return payload;
  }
  return {
    ...payload,
    latest_received_at: previous.latest_received_at,
    latest_value: previous.latest_value,
  };
}

function renderCard(
  card: HTMLElement,
  payload: HistorySeries,
  now: number,
  staleAfterMs: number,
  sessionStartedAt: number,
): void {
  const kind = card.dataset.valueKind ?? payload.value_type;
  const boolean = isBooleanKind(kind);
  const unit = card.dataset.unit ?? payload.unit;
  const decimalPlaces = Number(card.dataset.decimalPlaces ?? 1);
  const value = query<HTMLElement>("[data-live-value]", card);
  const received = query<HTMLElement>("[data-live-received]", card);
  const summary = query<HTMLElement>("[data-live-summary]", card);
  const chart = query<SVGSVGElement>("[data-live-chart]", card);
  const pointCount = chart
    ? renderChart(chart, payload, kind, unit, now, sessionStartedAt)
    : 0;
  if (payload.latest_received_at === null) {
    setStatus(card, "未受信", "never");
    if (value) value.textContent = "—";
    if (received) received.textContent = "まだ受信していません";
  } else {
    const relative = relativeTime(payload.latest_received_at, now);
    const stale = now - payload.latest_received_at > staleAfterMs;
    setStatus(card, stale ? "要確認" : "受信中", stale ? "stale" : "receiving");
    if (received) {
      received.textContent = `最終受信 ${relative}`;
      received.title = new Date(payload.latest_received_at).toLocaleString("ja-JP");
    }
    if (value) {
      if (
        boolean &&
        (typeof payload.latest_value === "boolean" ||
          typeof payload.latest_value === "number")
      ) {
        value.textContent =
          (typeof payload.latest_value === "boolean"
            ? payload.latest_value
            : payload.latest_value >= 0.5)
            ? "ON"
            : "OFF";
      } else if (typeof payload.latest_value === "number") {
        value.textContent = `${formatNumber(payload.latest_value, decimalPlaces)}${unit ? ` ${unit}` : ""}`;
      }
    }
  }
  if (summary) {
    summary.textContent = boolean
      ? `この画面を開いてから${pointCount}件の状態変化を表示しています。全期間・最大1,000bucket、縦軸はON/OFFです。`
      : `この画面を開いてから${pointCount}件を表示しています。全期間・最大1,000bucket、縦軸は値${unit ? `（${unit}）` : ""}です。`;
  }
}

function activeCards(dashboard: HTMLElement): HTMLElement[] {
  const viewportHeight = window.innerHeight || document.documentElement.clientHeight;
  return queryAll<HTMLElement>("[data-live-signal]", dashboard)
    .filter((card) => {
      const bounds = card.getBoundingClientRect();
      return bounds.bottom >= 0 && bounds.top <= viewportHeight;
    })
    .slice(0, MAX_ACTIVE_CARDS);
}

export function initializeLiveDashboard(): void {
  const dashboard = query<HTMLElement>("[data-live-dashboard]");
  const state = query<HTMLElement>("[data-live-dashboard-state]");
  if (!dashboard) return;
  const staleAfterMs = Number(dashboard.dataset.staleAfterMs ?? 300_000);
  const sessionStartedAt = Number(dashboard.dataset.liveSessionStartedAt);
  if (!Number.isFinite(sessionStartedAt)) {
    if (state) state.textContent = "ライブ更新を開始できません";
    return;
  }
  const pageOpenedAt = performance.now();
  const edgeNow = (): number =>
    Math.floor(sessionStartedAt + Math.max(0, performance.now() - pageOpenedAt));
  const snapshotAt = Number(dashboard.dataset.liveSnapshotAt);
  const liveSnapshotAt = Number.isFinite(snapshotAt) && snapshotAt >= 0 && snapshotAt <= sessionStartedAt
    ? snapshotAt
    : sessionStartedAt;
  const latestPayloads = new WeakMap<HTMLElement, HistorySeries>();
  const catchUpComplete = new WeakSet<HTMLElement>();
  let controller: AbortController | null = null;

  const refresh = async (): Promise<void> => {
    if (!dashboard.isConnected || document.visibilityState !== "visible") return;
    controller?.abort();
    controller = new AbortController();
    const now = edgeNow();
    const totalCards = queryAll<HTMLElement>("[data-live-signal]", dashboard).length;
    if (!totalCards) {
      if (state) state.textContent = "有効な計測ルールがありません。計測ルールを設定してください";
      return;
    }
    const cards = activeCards(dashboard);
    if (!cards.length) {
      if (state) state.textContent = "表示領域内の計測ルールを待っています";
      return;
    }
    for (const card of cards) {
      const cached = latestPayloads.get(card);
      if (cached) {
        renderCard(card, cached, now, staleAfterMs, sessionStartedAt);
      }
    }
    const results = await Promise.all(
      cards.map(async (card) => {
        const ruleId = card.dataset.ruleId;
        if (!ruleId) return false;
        const catchingUp = liveSnapshotAt < sessionStartedAt && !catchUpComplete.has(card);
        const requestFrom = catchingUp ? liveSnapshotAt : sessionStartedAt;
        const requestWindow = historyWindow(requestFrom, now + 1);
        const result = await getHistorySeries(
          ruleId,
          requestWindow.from,
          requestWindow.to,
          requestWindow.bucketMs,
          controller!.signal,
        ).catch(() => null);
        if (!result?.ok) return false;
        const payload = retainCardLatest(latestPayloads.get(card), result.value);
        const renderedPayload = catchingUp
          ? { ...payload, sample_count: 0, points: [] }
          : payload;
        if (catchingUp) catchUpComplete.add(card);
        latestPayloads.set(card, renderedPayload);
        renderCard(card, renderedPayload, now, staleAfterMs, sessionStartedAt);
        return true;
      }),
    );
    if (state) {
      const succeeded = results.filter(Boolean).length;
      state.textContent = succeeded === cards.length
        ? `自動更新中・${succeeded}件を確認`
        : `一部を確認できません・${succeeded}/${cards.length}件`;
    }
  };

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") void refresh();
    else controller?.abort();
  });
  if (document.visibilityState === "visible") void refresh();
  window.setInterval(() => void refresh(), REFRESH_MS);
}
