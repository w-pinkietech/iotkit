import { afterEach, describe, expect, it } from "vitest";
import { renderSignalChart } from "../../src/signal-chart";

afterEach(() => {
  document.body.replaceChildren();
});

describe("shared signal chart renderer", () => {
  it("renders boolean samples as a step path with shared chart classes", () => {
    const svg = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "svg",
    );
    document.body.append(svg);

    expect(
      renderSignalChart(svg, {
        points: [
          { at: 0, value: 0 },
          { at: 1_000, value: 1 },
          { at: 2_000, value: 1 },
          { at: 3_000, value: 0 },
        ],
        boolean: true,
        startAt: 0,
        endAt: 4_000,
      }),
    ).toBe(4);
    expect(svg.querySelector(".chart-grid")).not.toBeNull();
    expect(svg.querySelector(".chart-line-raw")?.getAttribute("d")).toMatch(
      / H .* V .* H .* V /,
    );
  });

  it("uses compact geometry for charts rendered inside live cards", () => {
    const svg = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "svg",
    );
    document.body.append(svg);

    expect(
      renderSignalChart(svg, {
        points: [],
        geometry: "compact",
      }),
    ).toBe(0);
    expect(svg.getAttribute("viewBox")).toBe("0 0 360 160");
    expect(svg.dataset.chartGeometry).toBe("compact");
    expect(svg.querySelector(".chart-empty-title")?.getAttribute("x")).toBe(
      "180",
    );
  });

  it("reserves compact width for four-digit y-axis labels", () => {
    const svg = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "svg",
    );
    document.body.append(svg);

    renderSignalChart(svg, {
      points: [
        { at: 0, value: 1_200 },
        { at: 1_000, value: 1_210 },
      ],
      geometry: "compact",
      startAt: 0,
      endAt: 2_000,
    });

    const yAxisLabels = Array.from(
      svg.querySelectorAll<SVGTextElement>(".chart-axis-label"),
    ).filter((label) => label.getAttribute("x") === "63");
    expect(yAxisLabels).toHaveLength(5);
  });

  it("updates existing chart elements without replacing the SVG DOM", () => {
    const svg = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "svg",
    );
    document.body.append(svg);

    renderSignalChart(svg, {
      points: [
        { at: 0, value: 1 },
        { at: 1_000, value: 2 },
      ],
      geometry: "compact",
      startAt: 0,
      endAt: 2_000,
    });
    const path = svg.querySelector(".chart-line-raw");
    const initialPath = path?.getAttribute("d");

    renderSignalChart(svg, {
      points: [
        { at: 0, value: 2 },
        { at: 1_000, value: 2 },
      ],
      geometry: "compact",
      startAt: 0,
      endAt: 2_000,
    });

    expect(svg.querySelector(".chart-line-raw")).toBe(path);
    expect(path?.getAttribute("d")).not.toBe(initialPath);
  });

  it("reconciles an optional middle series without duplicate children", () => {
    const svg = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "svg",
    );
    document.body.append(svg);
    const points = [
      { at: 0, value: 1, result: 10 },
      { at: 1_000, value: 2, result: 20 },
    ];
    const shapeSequence = (): string[] =>
      Array.from(svg.children).map(
        (child) => `${child.tagName}.${child.getAttribute("class") ?? ""}`,
      );

    renderSignalChart(svg, {
      points,
      geometry: "compact",
      startAt: 0,
      endAt: 2_000,
      showResult: false,
    });
    const withoutResult = shapeSequence();
    const rawPath = svg.querySelector(".chart-line-raw");

    renderSignalChart(svg, {
      points,
      geometry: "compact",
      startAt: 0,
      endAt: 2_000,
      showResult: true,
    });
    expect(svg.querySelectorAll(".chart-line-result")).toHaveLength(1);
    expect(shapeSequence()).toHaveLength(withoutResult.length + 1);

    renderSignalChart(svg, {
      points,
      geometry: "compact",
      startAt: 0,
      endAt: 2_000,
      showResult: false,
    });
    expect(shapeSequence()).toEqual(withoutResult);
    expect(svg.querySelectorAll(".chart-line-result")).toHaveLength(0);
    expect(svg.querySelector(".chart-line-raw")).toBe(rawPath);
  });
});
