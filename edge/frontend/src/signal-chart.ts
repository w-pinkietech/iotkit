/**
 * Shared SVG renderer for the compact signal charts used by the Console.
 *
 * The two callers deliberately prepare their data differently: the live view
 * uses persisted one-second buckets while the setting preview uses evaluated
 * input points.  Keeping the SVG geometry here makes their visual grammar the
 * same without making either data source depend on the other.
 */

const SVG_NS = "http://www.w3.org/2000/svg";

export interface SignalChartPoint {
  at: number;
  value: number;
  minimum?: number;
  maximum?: number;
  sampleCount?: number;
  result?: number;
  resultMinimum?: number;
  resultMaximum?: number;
  activeRatio?: number;
}

export interface SignalChartThresholds {
  rise?: number;
  fall?: number;
}

export interface SignalChartOptions {
  points: SignalChartPoint[];
  /** Shared geometry presets keep the preview spacious and live cards compact. */
  geometry?: "preview" | "compact";
  unit?: string;
  startAt?: number;
  endAt?: number;
  latestAt?: number;
  /** Use a 0/1 ON/OFF axis for the raw series. */
  boolean?: boolean;
  /** Use a step path for the raw series without changing its y-axis. */
  rawStep?: boolean;
  /** Use a step path for the result series. */
  resultStep?: boolean;
  showResult?: boolean;
  showRanges?: boolean;
  showLatestMarker?: boolean;
  showActiveBands?: boolean;
  thresholds?: SignalChartThresholds;
  axisLabels?: {
    start?: string;
    end?: string;
  };
  title?: string;
  emptyTitle?: string;
  emptyHint?: string;
}

interface ChartGeometry {
  width: number;
  height: number;
  left: number;
  right: number;
  top: number;
  bottom: number;
}

const GEOMETRIES: Record<"preview" | "compact", ChartGeometry> = {
  preview: {
    width: 760,
    height: 260,
    left: 58,
    right: 18,
    top: 18,
    bottom: 42,
  },
  compact: {
    width: 360,
    height: 160,
    left: 72,
    right: 12,
    top: 12,
    bottom: 28,
  },
};

function addSVG<K extends keyof SVGElementTagNameMap>(
  parent: SVGElement,
  name: K,
  attributes: Record<string, string | number> = {},
): SVGElementTagNameMap[K] {
  const element = document.createElementNS(SVG_NS, name);
  for (const [key, value] of Object.entries(attributes)) {
    element.setAttribute(key, String(value));
  }
  parent.append(element);
  return element;
}

function finite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function numberLabel(value: number): string {
  return value.toLocaleString("ja-JP", { maximumFractionDigits: 3 });
}

function drawText(
  svg: SVGSVGElement,
  text: string,
  attributes: Record<string, string | number>,
): void {
  const element = addSVG(svg, "text", attributes);
  element.textContent = text;
}

function emptyChart(
  svg: SVGSVGElement,
  title: string,
  hint: string,
  geometry: ChartGeometry,
): void {
  drawText(svg, title, {
    x: geometry.width / 2,
    y: geometry.height / 2 - 8,
    "text-anchor": "middle",
    class: "chart-empty-title live-chart-empty",
  });
  drawText(svg, hint, {
    x: geometry.width / 2,
    y: geometry.height / 2 + 18,
    "text-anchor": "middle",
    class: "chart-empty-hint live-chart-empty-hint",
  });
}

function pathFor(
  points: SignalChartPoint[],
  x: (at: number) => number,
  y: (value: number) => number,
  value: (point: SignalChartPoint) => number | undefined,
  step: boolean,
): string {
  let path = "";
  let previous: number | undefined;
  points.forEach((point) => {
    const current = value(point);
    if (!finite(current)) return;
    const pointX = x(point.at).toFixed(2);
    const pointY = y(current).toFixed(2);
    if (!path) {
      path = `M ${pointX} ${pointY}`;
    } else if (step && finite(previous)) {
      path += ` H ${pointX}`;
      if (current !== previous) path += ` V ${pointY}`;
    } else {
      path += ` L ${pointX} ${pointY}`;
    }
    previous = current;
  });
  return path;
}

function sameShape(target: Element, source: Element): boolean {
  return target.tagName === source.tagName &&
    target.getAttribute("class") === source.getAttribute("class");
}

function syncElement(
  target: Element,
  source: Element,
  syncAttributes = true,
): void {
  if (syncAttributes) {
    for (const attribute of Array.from(target.attributes)) {
      if (!source.hasAttribute(attribute.name)) target.removeAttribute(attribute.name);
    }
    for (const attribute of Array.from(source.attributes)) {
      if (target.getAttribute(attribute.name) !== attribute.value) {
        target.setAttribute(attribute.name, attribute.value);
      }
    }
  }
  const sourceChildren = Array.from(source.children);
  if (!sourceChildren.length) {
    if (target.textContent !== source.textContent) {
      target.textContent = source.textContent;
    }
    return;
  }
  for (const child of Array.from(target.childNodes)) {
    if (child.nodeType !== Node.ELEMENT_NODE) child.remove();
  }
  sourceChildren.forEach((sourceChild, index) => {
    const targetChild = target.children[index];
    if (targetChild && sameShape(targetChild, sourceChild)) {
      syncElement(targetChild, sourceChild);
      return;
    }
    target.insertBefore(sourceChild.cloneNode(true), targetChild ?? null);
  });
  while (target.children.length > sourceChildren.length) {
    target.lastElementChild?.remove();
  }
}

function syncChartDOM(target: SVGSVGElement, source: SVGSVGElement): void {
  target.setAttribute("viewBox", source.getAttribute("viewBox") ?? "");
  const geometry = source.dataset.chartGeometry;
  if (geometry) target.dataset.chartGeometry = geometry;
  syncElement(target, source, false);
}

function renderSignalChartDraft(
  svg: SVGSVGElement,
  options: SignalChartOptions,
): number {
  const geometry = GEOMETRIES[options.geometry ?? "preview"];
  const plotWidth = geometry.width - geometry.left - geometry.right;
  const plotHeight = geometry.height - geometry.top - geometry.bottom;
  svg.setAttribute(
    "viewBox",
    `0 0 ${geometry.width} ${geometry.height}`,
  );
  svg.dataset.chartGeometry = options.geometry ?? "preview";
  const points = options.points.filter(
    (point) => finite(point.at) && finite(point.value),
  );
  if (!points.length) {
    emptyChart(
      svg,
      options.emptyTitle ?? "まだ受信データがありません",
      options.emptyHint ?? "実際に届いた値を待っています",
      geometry,
    );
    return 0;
  }

  const startAt = finite(options.startAt) ? options.startAt : points[0].at;
  const latestPointAt = points.at(-1)?.at ?? startAt;
  const requestedEnd = finite(options.endAt) ? options.endAt : latestPointAt;
  const endAt = Math.max(startAt + 1_000, requestedEnd);
  const x = (at: number): number =>
    geometry.left +
    Math.max(0, Math.min(1, (at - startAt) / (endAt - startAt))) *
      plotWidth;

  const rawValues = points.flatMap((point) => [
    finite(point.minimum) ? point.minimum : point.value,
    finite(point.maximum) ? point.maximum : point.value,
  ]);
  const resultValues = options.showResult
    ? points.flatMap((point) =>
        [point.resultMinimum, point.resultMaximum, point.result].filter(finite),
      )
    : [];
  const thresholdValues = options.thresholds
    ? [options.thresholds.rise, options.thresholds.fall].filter(finite)
    : [];
  const values = [...rawValues, ...resultValues, ...thresholdValues];
  let minimum = options.boolean ? 0 : Math.min(...values);
  let maximum = options.boolean ? 1 : Math.max(...values);
  if (!finite(minimum) || !finite(maximum)) {
    minimum = 0;
    maximum = 1;
  }
  if (minimum === maximum) {
    const padding = Math.max(1, Math.abs(minimum) * 0.1);
    minimum -= padding;
    maximum += padding;
  } else if (!options.boolean) {
    const padding = (maximum - minimum) * 0.08;
    minimum -= padding;
    maximum += padding;
  }
  const y = (value: number): number =>
    geometry.top + ((maximum - value) * plotHeight) / (maximum - minimum);

  for (let index = 0; index <= 4; index += 1) {
    const gridY = geometry.top + (index * plotHeight) / 4;
    addSVG(svg, "line", {
      x1: geometry.left,
      x2: geometry.width - geometry.right,
      y1: gridY,
      y2: gridY,
      class: "chart-grid",
    });
    drawText(
      svg,
      options.boolean
        ? index === 0
          ? "ON"
          : index === 4
            ? "OFF"
            : ""
        : numberLabel(maximum - (index * (maximum - minimum)) / 4),
      {
        x: geometry.left - 9,
        y: gridY + 4,
        "text-anchor": "end",
        class: "chart-axis-label",
      },
    );
  }

  const drawThreshold = (value: number | undefined, labelText: string): void => {
    if (!finite(value)) return;
    const thresholdY = y(value);
    addSVG(svg, "line", {
      x1: geometry.left,
      x2: geometry.width - geometry.right,
      y1: thresholdY,
      y2: thresholdY,
      class: "chart-threshold",
    });
    drawText(svg, `${labelText} ${numberLabel(value)}`, {
      x: geometry.width - geometry.right - 4,
      y: thresholdY - 6,
      "text-anchor": "end",
      class: "chart-threshold-label",
    });
  };
  drawThreshold(options.thresholds?.rise, "立上り");
  drawThreshold(options.thresholds?.fall, "立下り");

  if (options.showRanges !== false) {
    points.forEach((point) => {
      const sampleCount = point.sampleCount ?? 1;
      if (sampleCount <= 1) return;
      const minimumValue = finite(point.minimum) ? point.minimum : point.value;
      const maximumValue = finite(point.maximum) ? point.maximum : point.value;
      addSVG(svg, "line", {
        x1: x(point.at),
        x2: x(point.at),
        y1: y(minimumValue),
        y2: y(maximumValue),
        class: "chart-range",
      });
      if (options.showResult) {
        const resultMinimum = finite(point.resultMinimum)
          ? point.resultMinimum
          : point.result;
        const resultMaximum = finite(point.resultMaximum)
          ? point.resultMaximum
          : point.result;
        if (finite(resultMinimum) && finite(resultMaximum)) {
          addSVG(svg, "line", {
            x1: x(point.at) + 2,
            x2: x(point.at) + 2,
            y1: y(resultMinimum),
            y2: y(resultMaximum),
            class: "chart-range-result",
          });
        }
      }
    });
  }

  if (options.showActiveBands) {
    points.forEach((point) => {
      const ratio = Math.max(0, Math.min(1, point.activeRatio ?? 0));
      if (!ratio) return;
      addSVG(svg, "rect", {
        x: x(point.at) - Math.max(1, plotWidth / Math.max(points.length, 1) / 2),
        y: geometry.top,
        width: Math.max(2, plotWidth / Math.max(points.length, 1)),
        height: plotHeight,
        class: "chart-active-band",
        opacity: Math.max(0.12, ratio * 0.24),
      });
    });
  }

  const rawPath = pathFor(
    points,
    x,
    y,
    (point) => (options.boolean ? (point.value >= 0.5 ? 1 : 0) : point.value),
    options.rawStep ?? Boolean(options.boolean),
  );
  if (rawPath) {
    addSVG(svg, "path", {
      d: rawPath,
      class: "chart-line chart-line-raw live-chart-line",
    });
  }
  if (options.showResult) {
    const resultPath = pathFor(
      points,
      x,
      y,
      (point) => point.result,
      Boolean(options.resultStep),
    );
    if (resultPath) {
      addSVG(svg, "path", {
        d: resultPath,
        class: "chart-line chart-line-result live-chart-result-line",
      });
    }
  }

  const latestPoint = points.at(-1);
  if (options.showLatestMarker && latestPoint) {
    const latestAt = finite(options.latestAt) ? options.latestAt : latestPoint.at;
    const latestValue = options.showResult && finite(latestPoint.result)
      ? latestPoint.result
      : options.boolean
        ? latestPoint.value >= 0.5
          ? 1
          : 0
        : latestPoint.value;
    const latestX = x(latestAt);
    const latestY = y(latestValue);
    addSVG(svg, "line", {
      x1: latestX,
      x2: latestX,
      y1: latestY,
      y2: geometry.top + plotHeight,
      class: "chart-latest-guide live-chart-latest-guide",
    });
    addSVG(svg, "circle", {
      cx: latestX,
      cy: latestY,
      r: 5,
      class: "chart-latest-point live-chart-latest-point",
    });
    drawText(svg, "最新", {
      x: Math.min(geometry.width - geometry.right - 4, latestX - 8),
      y: Math.max(geometry.top + 13, latestY - 10),
      "text-anchor": "end",
      class: "chart-latest-label live-chart-latest-label",
    });
  }

  const startLabel = options.axisLabels?.start ?? new Date(startAt).toLocaleTimeString("ja-JP", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  const endLabel = options.axisLabels?.end ?? new Date(endAt).toLocaleTimeString("ja-JP", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  drawText(svg, startLabel, {
    x: geometry.left,
    y: geometry.height - 14,
    class: "chart-axis-label",
  });
  drawText(svg, endLabel, {
    x: geometry.width - geometry.right,
    y: geometry.height - 14,
    "text-anchor": "end",
    class: "chart-axis-label",
  });
  const title = addSVG(svg, "title");
  title.textContent = options.title ??
    `横軸は直近の時間、縦軸は値${options.unit ? `（${options.unit}）` : ""}です。`;
  return points.length;
}

/** Render a signal chart and return the number of plotted points. */
export function renderSignalChart(
  svg: SVGSVGElement,
  options: SignalChartOptions,
): number {
  const draft = document.createElementNS(SVG_NS, "svg");
  const plottedCount = renderSignalChartDraft(draft, options);
  syncChartDOM(svg, draft);
  return plottedCount;
}
