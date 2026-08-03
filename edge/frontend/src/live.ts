import { getHistorySeries, type HistorySeries } from "./api";
import { query, queryAll } from "./dom";

const WINDOW_MS = 15 * 60 * 1_000;
const BUCKET_MS = 15 * 1_000;
const REFRESH_MS = 5 * 1_000;
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
  return kind === "bool" || kind === "boolean";
}

function relativeTime(receivedAt: number, now: number): string {
  const elapsed = Math.max(0, now - receivedAt);
  if (elapsed < 10_000) return "たった今";
  if (elapsed < 60_000) return `${Math.floor(elapsed / 1_000)}秒前`;
  if (elapsed < 60 * 60_000) return `${Math.floor(elapsed / 60_000)}分前`;
  return `${Math.floor(elapsed / (60 * 60_000))}時間前`;
}

function renderEmpty(svg: SVGSVGElement, boolean: boolean): void {
  svg.replaceChildren();
  const label = addSVG(svg, "text", {
    x: 180,
    y: 82,
    "text-anchor": "middle",
    class: "live-chart-empty",
  });
  label.textContent = boolean ? "接点データを待っています" : "数値データを待っています";
}

function renderChart(
  svg: SVGSVGElement,
  payload: HistorySeries,
  boolean: boolean,
  unit: string,
  now: number,
): void {
  svg.replaceChildren();
  const points = payload.points;
  if (!points.length) {
    renderEmpty(svg, boolean);
    return;
  }
  const width = 360;
  const height = 160;
  const left = 42;
  const right = 12;
  const top = 12;
  const bottom = 28;
  const plotWidth = width - left - right;
  const plotHeight = height - top - bottom;
  const windowStart = now - WINDOW_MS;
  const x = (time: number): number =>
    left +
    Math.max(0, Math.min(1, (time - windowStart) / WINDOW_MS)) * plotWidth;
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
  startLabel.textContent = "15分前";
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
  const title = addSVG(svg, "title", {});
  title.textContent = boolean
    ? "横軸は直近15分、縦軸は接点のON/OFFです。"
    : `横軸は直近15分、縦軸は値${unit ? `（${unit}）` : ""}です。`;
}

function setStatus(card: HTMLElement, label: string, className: string): void {
  const status = query<HTMLElement>("[data-live-status]", card);
  if (!status) return;
  status.textContent = label;
  status.className = `status-pill ${className}`;
}

function renderCard(card: HTMLElement, payload: HistorySeries, now: number, staleAfterMs: number): void {
  const kind = card.dataset.valueKind ?? payload.value_type;
  const boolean = isBooleanKind(kind);
  const unit = card.dataset.unit ?? payload.unit;
  const decimalPlaces = Number(card.dataset.decimalPlaces ?? 1);
  const value = query<HTMLElement>("[data-live-value]", card);
  const received = query<HTMLElement>("[data-live-received]", card);
  const summary = query<HTMLElement>("[data-live-summary]", card);
  const chart = query<SVGSVGElement>("[data-live-chart]", card);
  if (chart) renderChart(chart, payload, boolean, unit, now);
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
      ? `${payload.sample_count}件を表示しています。横軸は直近15分、縦軸はON/OFFです。`
      : `${payload.sample_count}件を表示しています。横軸は直近15分、縦軸は値${unit ? `（${unit}）` : ""}です。`;
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
  let controller: AbortController | null = null;

  const refresh = async (): Promise<void> => {
    if (!dashboard.isConnected || document.visibilityState !== "visible") return;
    controller?.abort();
    controller = new AbortController();
    const now = Date.now();
    const cards = activeCards(dashboard);
    if (!cards.length) return;
    const results = await Promise.all(
      cards.map(async (card) => {
        const signalRef = card.dataset.signalRef;
        if (!signalRef) return false;
        const result = await getHistorySeries(
          signalRef,
          now - WINDOW_MS,
          now + 1,
          BUCKET_MS,
          controller!.signal,
        ).catch(() => null);
        if (!result?.ok) return false;
        renderCard(card, result.value, now, staleAfterMs);
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
