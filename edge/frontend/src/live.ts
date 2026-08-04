import { getHistorySeries, type HistorySeries } from "./api";
import { query, queryAll } from "./dom";

const REFRESH_MS = 5 * 1_000;
const SESSION_WINDOW_MS = 5 * 60 * 1_000;
const BUCKET_MS = REFRESH_MS;
const MAX_NUMERIC_POINTS = 60;
const MAX_BOOLEAN_POINTS = 10;
const MAX_ACTIVE_CARDS = 12;
const SVG_NS = "http://www.w3.org/2000/svg";

function addSVG<K extends keyof SVGElementTagNameMap>(
  svg: SVGSVGElement,
  tag: K,
  attributes: Record<string, string | number>,
): SVGElementTagNameMap[K] {
  const element = document.createElementNS(SVG_NS, tag);
  for (const [name, value] of Object.entries(attributes)) {
    element.setAttribute(name, String(value));
  }
  svg.append(element);
  return element;
}

function formatNumber(value: number, decimalPlaces = 1): string {
  return value.toLocaleString("ja-JP", {
    maximumFractionDigits: Math.max(0, decimalPlaces),
  });
}

function isBooleanKind(kind: string): boolean {
  return kind === "bool" || kind === "boolean" || kind === "alarm";
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

function renderEmpty(svg: SVGSVGElement): void {
  svg.replaceChildren();
  const label = addSVG(svg, "text", {
    x: 180,
    y: 82,
    "text-anchor": "middle",
    class: "live-chart-empty",
  });
  label.textContent = "この画面を開いてからの受信を待っています";
}

function sessionPoints(
  payload: HistorySeries,
  boolean: boolean,
  windowStart: number,
): HistorySeries["points"] {
  const points = payload.points.filter((point) => point.bucket_start >= windowStart);
  if (!boolean) return points.slice(-MAX_NUMERIC_POINTS);
  const transitions: HistorySeries["points"] = [];
  for (const point of points) {
    const state = point.average >= 0.5 ? 1 : 0;
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
  boolean: boolean,
  unit: string,
  now: number,
  sessionStartedAt: number,
): number {
  svg.replaceChildren();
  const windowStart = Math.max(sessionStartedAt, now - SESSION_WINDOW_MS);
  const points = sessionPoints(payload, boolean, windowStart);
  if (!points.length) {
    renderEmpty(svg);
    return 0;
  }
  const width = 360;
  const height = 160;
  const left = 42;
  const right = 12;
  const top = 12;
  const bottom = 28;
  const plotWidth = width - left - right;
  const plotHeight = height - top - bottom;
  const windowEnd = Math.max(now, windowStart + REFRESH_MS);
  const x = (time: number): number =>
    left +
    Math.max(0, Math.min(1, (time - windowStart) / (windowEnd - windowStart))) * plotWidth;
  const sourceValues = points.flatMap((point) => [point.minimum, point.maximum]);
  let minimum = boolean ? 0 : Math.min(...sourceValues);
  let maximum = boolean ? 1 : Math.max(...sourceValues);
  if (!boolean) {
    const padding = minimum === maximum
      ? Math.max(1, Math.abs(minimum) * 0.1)
      : (maximum - minimum) * 0.08;
    minimum -= padding;
    maximum += padding;
  }
  const y = (value: number): number =>
    top + ((maximum - value) * plotHeight) / (maximum - minimum);

  for (const ratio of [0, 0.5, 1]) {
    const gridY = top + ratio * plotHeight;
    addSVG(svg, "line", {
      x1: left,
      x2: width - right,
      y1: gridY,
      y2: gridY,
      class: "live-chart-grid",
    });
  }
  for (const [value, label] of boolean
    ? [[1, "ON"], [0, "OFF"]] as Array<[number, string]>
    : [[maximum, formatNumber(maximum)], [minimum, formatNumber(minimum)]] as Array<[number, string]>) {
    const text = addSVG(svg, "text", {
      x: left - 7,
      y: y(value) + 4,
      "text-anchor": "end",
      class: "live-chart-axis-label",
    });
    text.textContent = label;
  }
  const startLabel = addSVG(svg, "text", {
    x: left,
    y: height - 7,
    "text-anchor": "start",
    class: "live-chart-axis-label",
  });
  startLabel.textContent = windowStart === sessionStartedAt ? "開始" : "5分前";
  const endLabel = addSVG(svg, "text", {
    x: width - right,
    y: height - 7,
    "text-anchor": "end",
    class: "live-chart-axis-label",
  });
  endLabel.textContent = "現在";

  let path = "";
  points.forEach((point, index) => {
    const pointX = x(point.bucket_start).toFixed(2);
    const pointY = y(boolean ? (point.average >= 0.5 ? 1 : 0) : point.average).toFixed(2);
    if (index === 0) path = `M ${pointX} ${pointY}`;
    else if (boolean) path += ` H ${pointX} V ${pointY}`;
    else path += ` L ${pointX} ${pointY}`;
  });
  addSVG(svg, "path", { d: path, class: "live-chart-line" });
  const latest = points.at(-1)!;
  const latestX = x(
    payload.latest_received_at !== null && payload.latest_received_at >= windowStart
      ? payload.latest_received_at
      : latest.bucket_start,
  );
  const latestY = y(boolean ? (latest.average >= 0.5 ? 1 : 0) : latest.average);
  addSVG(svg, "line", {
    x1: latestX,
    x2: latestX,
    y1: latestY,
    y2: top + plotHeight,
    class: "live-chart-latest-guide",
  });
  addSVG(svg, "circle", {
    cx: latestX,
    cy: latestY,
    r: 4,
    class: "live-chart-latest-point",
  });
  const latestLabel = addSVG(svg, "text", {
    x: latestX + (latestX > width - 82 ? -7 : 7),
    y: Math.max(top + 10, latestY - 7),
    "text-anchor": latestX > width - 82 ? "end" : "start",
    class: "live-chart-latest-label",
  });
  latestLabel.textContent = "最終データ";
  const title = addSVG(svg, "title", {});
  title.textContent = boolean
    ? "横軸はこの画面を開いてから（最大5分）、縦軸は接点のON/OFFです。"
    : `横軸はこの画面を開いてから（最大5分）、縦軸は値${unit ? `（${unit}）` : ""}です。`;
  return points.length;
}

function setStatus(card: HTMLElement, label: string, className: string): void {
  const status = query<HTMLElement>("[data-live-status]", card);
  if (!status) return;
  status.textContent = label;
  status.className = `status-pill ${className}`;
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
    ? renderChart(chart, payload, boolean, unit, now, sessionStartedAt)
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
      ? `この画面を開いてから${pointCount}件を表示しています。横軸は開始から現在（最大5分）、縦軸はON/OFFです。`
      : `この画面を開いてから${pointCount}件を表示しています。横軸は開始から現在（最大5分）、縦軸は値${unit ? `（${unit}）` : ""}です。`;
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
  const latestPayloads = new WeakMap<HTMLElement, HistorySeries>();
  let controller: AbortController | null = null;

  const refresh = async (): Promise<void> => {
    if (!dashboard.isConnected || document.visibilityState !== "visible") return;
    controller?.abort();
    controller = new AbortController();
    const now = edgeNow();
    const cards = activeCards(dashboard);
    if (!cards.length) {
      if (state) state.textContent = "有効な計測ルールがありません。計測ルールを設定してください";
      return;
    }
    for (const card of cards) {
      const cached = latestPayloads.get(card);
      if (cached) renderCard(card, cached, now, staleAfterMs, sessionStartedAt);
    }
    const results = await Promise.all(
      cards.map(async (card) => {
        const ruleId = card.dataset.ruleId;
        if (!ruleId) return false;
        const result = await getHistorySeries(
          ruleId,
          Math.max(sessionStartedAt, now - SESSION_WINDOW_MS),
          now + 1,
          BUCKET_MS,
          controller!.signal,
        ).catch(() => null);
        if (!result?.ok) return false;
        latestPayloads.set(card, result.value);
        renderCard(card, result.value, now, staleAfterMs, sessionStartedAt);
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
