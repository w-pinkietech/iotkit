import { afterEach, describe, expect, it, vi } from "vitest";
import { initializeLiveDashboard } from "../../src/live";

function card(
  signalRef: string,
  kind: "numeric" | "boolean",
  unit = "",
): string {
  return `
    <article data-live-signal data-signal-ref="${signalRef}"
      data-value-kind="${kind}" data-decimal-places="1" data-unit="${unit}">
      <span data-live-status>確認中</span>
      <strong data-live-value>—</strong>
      <span data-live-received>最終受信を確認中</span>
      <svg data-live-chart></svg>
      <span data-live-summary></span>
    </article>
  `;
}

function responseFor(signalRef: string): Response {
  const boolean = signalRef === "contact-01";
  return new Response(
    JSON.stringify({
      signal_ref: signalRef,
      display_name: boolean ? "運転接点" : "炉内温度",
      unit: boolean ? "" : "℃",
      value_type: boolean ? "boolean" : "number",
      sample_count: 3,
      latest_received_at: 1_700_000_000_000,
      latest_value: boolean ? true : 24.8,
      points: boolean
        ? [
            { bucket_start: 1_699_999_970_000, minimum: 0, average: 0, maximum: 0, sample_count: 1 },
            { bucket_start: 1_699_999_985_000, minimum: 1, average: 1, maximum: 1, sample_count: 1 },
            { bucket_start: 1_700_000_000_000, minimum: 1, average: 1, maximum: 1, sample_count: 1 },
          ]
        : [
            { bucket_start: 1_699_999_970_000, minimum: 22, average: 22, maximum: 22, sample_count: 1 },
            { bucket_start: 1_699_999_985_000, minimum: 23, average: 23, maximum: 23, sample_count: 1 },
            { bucket_start: 1_700_000_000_000, minimum: 24.8, average: 24.8, maximum: 24.8, sample_count: 1 },
          ],
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  );
}

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("live dashboard", () => {
  it("renders numeric lines and boolean steps with current values and axes", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_699_999_960_000);
    document.body.innerHTML = `
      <section data-live-dashboard>
        <span data-live-dashboard-state>更新を準備中</span>
        ${card("temperature-01", "numeric", "℃")}
        ${card("contact-01", "boolean")}
      </section>
    `;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = new URL(String(input), "http://localhost");
        return Promise.resolve(responseFor(url.searchParams.get("signal_ref") ?? ""));
      }),
    );

    initializeLiveDashboard();
    await vi.advanceTimersByTimeAsync(0);

    const numeric = document.querySelector<HTMLElement>(
      '[data-signal-ref="temperature-01"]',
    )!;
    const contact = document.querySelector<HTMLElement>(
      '[data-signal-ref="contact-01"]',
    )!;
    expect(numeric.querySelector("[data-live-value]")?.textContent).toBe("24.8 ℃");
    expect(numeric.querySelector("path")?.getAttribute("d")).toContain(" L ");
    expect(numeric.querySelector("[data-live-summary]")?.textContent).toContain(
      "縦軸は値",
    );
    expect(contact.querySelector("[data-live-value]")?.textContent).toBe("ON");
    expect(contact.querySelector("path")?.getAttribute("d")).toMatch(/H .* V /);
    expect(contact.querySelector("path")?.getAttribute("d")?.match(/ V /g)).toHaveLength(1);
    expect(contact.querySelector("[data-live-summary]")?.textContent).toContain(
      "ON/OFF",
    );
    expect(document.querySelector("[data-live-dashboard-state]")?.textContent).toContain(
      "自動更新中",
    );
  });

  it("starts empty and plots only data received after the live view opens", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_700_000_005_000);
    document.body.innerHTML = `
      <section data-live-dashboard>
        <span data-live-dashboard-state>更新を準備中</span>
        ${card("temperature-01", "numeric", "℃")}
      </section>
    `;
    const historical = await responseFor("temperature-01").json();
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(historical)))
      .mockResolvedValue(new Response(JSON.stringify({
        ...historical,
        sample_count: 1,
        latest_received_at: 1_700_000_010_000,
        latest_value: 25.2,
        points: [
          ...historical.points,
          {
            bucket_start: 1_700_000_010_000,
            minimum: 25.2,
            average: 25.2,
            maximum: 25.2,
            sample_count: 1,
          },
        ],
      })));
    vi.stubGlobal("fetch", fetch);

    initializeLiveDashboard();
    await vi.advanceTimersByTimeAsync(0);

    const liveCard = document.querySelector<HTMLElement>(
      '[data-signal-ref="temperature-01"]',
    )!;
    expect(liveCard.querySelector("path")).toBeNull();
    expect(liveCard.querySelector(".live-chart-empty")?.textContent).toBe(
      "この画面を開いてからの受信を待っています",
    );
    const firstRequest = new URL(String(fetch.mock.calls[0]?.[0]), "http://localhost");
    expect(firstRequest.searchParams.get("from")).toBe("1700000005000");
    expect(firstRequest.searchParams.get("bucket_ms")).toBe("5000");

    await vi.advanceTimersByTimeAsync(5_000);

    expect(liveCard.querySelector("path")?.getAttribute("d")).toMatch(/^M /);
    expect(liveCard.querySelector(".live-chart-latest-point")).not.toBeNull();
    expect(liveCard.querySelector("[data-live-summary]")?.textContent).toContain(
      "この画面を開いてから1件",
    );
  });

  it("bounds numeric samples to sixty and contact transitions to ten", async () => {
    vi.useFakeTimers();
    const sessionStart = 1_700_000_000_000;
    vi.setSystemTime(sessionStart);
    document.body.innerHTML = `
      <section data-live-dashboard>
        <span data-live-dashboard-state></span>
        ${card("temperature-01", "numeric", "℃")}
        ${card("contact-01", "boolean")}
      </section>
    `;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const signalRef = new URL(String(input), "http://localhost").searchParams.get(
          "signal_ref",
        )!;
        const boolean = signalRef === "contact-01";
        const points = Array.from({ length: 65 }, (_, index) => ({
          bucket_start: sessionStart + index * 5_000,
          minimum: boolean ? index % 2 : index,
          average: boolean ? index % 2 : index,
          maximum: boolean ? index % 2 : index,
          sample_count: 1,
        }));
        return Promise.resolve(new Response(JSON.stringify({
          signal_ref: signalRef,
          display_name: boolean ? "運転接点" : "炉内温度",
          unit: boolean ? "" : "℃",
          value_type: boolean ? "boolean" : "number",
          sample_count: points.length,
          latest_received_at: points.at(-1)!.bucket_start,
          latest_value: points.at(-1)!.average,
          points,
        })));
      }),
    );

    initializeLiveDashboard();
    await vi.advanceTimersByTimeAsync(0);

    const numericPath = document.querySelector<SVGPathElement>(
      '[data-signal-ref="temperature-01"] path',
    )!.getAttribute("d")!;
    const contactPath = document.querySelector<SVGPathElement>(
      '[data-signal-ref="contact-01"] path',
    )!.getAttribute("d")!;
    expect(numericPath.match(/ L /g)).toHaveLength(59);
    expect(contactPath.match(/ V /g)).toHaveLength(9);
    expect(
      document.querySelector('[data-signal-ref="temperature-01"] [data-live-summary]')
        ?.textContent,
    ).toContain("60件");
    expect(
      document.querySelector('[data-signal-ref="contact-01"] [data-live-summary]')
        ?.textContent,
    ).toContain("10件");
  });

  it("keeps the received age moving when a refresh fails", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_700_000_125_000);
    document.body.innerHTML = `
      <section data-live-dashboard>
        <span data-live-dashboard-state>更新を準備中</span>
        ${card("temperature-01", "numeric", "℃")}
      </section>
    `;
    const initialPayload = await responseFor("temperature-01").json();
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({
        ...initialPayload,
        latest_received_at: 1_700_000_002_500,
      })))
      .mockRejectedValue(new Error("offline"));
    vi.stubGlobal("fetch", fetch);

    initializeLiveDashboard();
    await vi.advanceTimersByTimeAsync(0);

    const liveCard = document.querySelector<HTMLElement>(
      '[data-signal-ref="temperature-01"]',
    )!;
    expect(liveCard.querySelector("[data-live-received]")?.textContent).toBe(
      "最終受信 2分2秒前",
    );
    expect(liveCard.querySelector(".live-chart-latest-point")).toBeNull();

    await vi.advanceTimersByTimeAsync(5_000);

    expect(liveCard.querySelector("[data-live-received]")?.textContent).toBe(
      "最終受信 2分7秒前",
    );
    expect(document.querySelector("[data-live-dashboard-state]")?.textContent).toContain(
      "一部を確認できません",
    );
  });

  it("does not poll hidden documents and bounds one cycle to twelve cards", async () => {
    vi.useFakeTimers();
    document.body.innerHTML = `
      <section data-live-dashboard>
        <span data-live-dashboard-state></span>
        ${Array.from({ length: 13 }, (_, index) => card(`signal-${index}`, "numeric")).join("")}
      </section>
    `;
    const fetch = vi.fn((input: RequestInfo | URL) => {
      const url = new URL(String(input), "http://localhost");
      return Promise.resolve(responseFor(url.searchParams.get("signal_ref") ?? ""));
    });
    vi.stubGlobal("fetch", fetch);
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });

    initializeLiveDashboard();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(fetch).not.toHaveBeenCalled();

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(0);
    expect(fetch).toHaveBeenCalledTimes(12);
  });

  it("distinguishes never received data from an advisory stale state", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_700_000_600_000);
    document.body.innerHTML = `
      <section data-live-dashboard data-stale-after-ms="300000">
        <span data-live-dashboard-state></span>
        ${card("stale-01", "numeric", "℃")}
        ${card("never-01", "boolean")}
      </section>
    `;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const signalRef = new URL(String(input), "http://localhost").searchParams.get(
          "signal_ref",
        )!;
        const base = await responseFor("temperature-01").json();
        return new Response(
          JSON.stringify({
            ...base,
            signal_ref: signalRef,
            latest_received_at:
              signalRef === "stale-01" ? 1_700_000_000_000 : null,
            latest_value: signalRef === "stale-01" ? 24.8 : null,
            points: signalRef === "stale-01" ? base.points : [],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }),
    );

    initializeLiveDashboard();
    await vi.advanceTimersByTimeAsync(0);

    const stale = document.querySelector<HTMLElement>('[data-signal-ref="stale-01"]')!;
    const never = document.querySelector<HTMLElement>('[data-signal-ref="never-01"]')!;
    expect(stale.querySelector("[data-live-status]")?.textContent).toBe("要確認");
    expect(stale.querySelector("[data-live-status]")?.classList.contains("stale")).toBe(true);
    expect(never.querySelector("[data-live-status]")?.textContent).toBe("未受信");
    expect(never.querySelector("[data-live-value]")?.textContent).toBe("—");
  });
});
