import { afterEach, describe, expect, it, vi } from "vitest";
import { initializeShell } from "../../src/shell";

afterEach(() => {
  document.body.replaceChildren();
  document.body.removeAttribute("data-focus-target");
  document.body.removeAttribute("data-activation-refresh");
  vi.restoreAllMocks();
  vi.clearAllTimers();
  vi.useRealTimers();
  sessionStorage.clear();
  window.history.replaceState(null, "", "/");
});

describe("console shell", () => {
  it("reloads an activation view every three seconds with a bounded retry count", async () => {
    vi.useFakeTimers();
    document.body.dataset.activationRefresh = "true";
    document.body.innerHTML = `
      <div data-activation-feedback>
        <strong data-activation-state>登録状態を自動確認中</strong>
        <span data-activation-guidance>
          3秒ごとに登録状態を自動確認します。サーバー側の登録処理は続きます。
        </span>
        <time data-activation-checked-at>画面表示時</time>
        <button type="button" data-activation-check-now>今すぐ確認</button>
      </div>
    `;
    const reload = vi.fn();

    initializeShell(reload);
    expect(
      document.querySelector("time")?.textContent,
    ).toMatch(/^\d{2}:\d{2}:\d{2}$/);
    await vi.advanceTimersByTimeAsync(2_999);
    expect(reload).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(reload).toHaveBeenCalledOnce();
    expect(sessionStorage.getItem("iotkit-activation-refresh:/")).toBe("1");

    vi.clearAllTimers();
    sessionStorage.setItem("iotkit-activation-refresh:/", "20");
    initializeShell(reload);
    expect(
      document.querySelector("[data-activation-state]")?.textContent,
    ).toBe("自動確認を一時停止しました");
    expect(
      document.querySelector("[data-activation-guidance]")?.textContent,
    ).toContain("サーバー側の登録処理は続いています");
    await vi.advanceTimersByTimeAsync(3_000);
    expect(reload).toHaveBeenCalledOnce();

    document
      .querySelector<HTMLButtonElement>("[data-activation-check-now]")!
      .click();
    expect(sessionStorage.getItem("iotkit-activation-refresh:/")).toBe("0");
    expect(reload).toHaveBeenCalledTimes(2);
  });

  it("does not reload an unrelated form while another activation is in progress", async () => {
    vi.useFakeTimers();
    document.body.innerHTML = `<form data-signal-profile></form>`;
    const reload = vi.fn();

    initializeShell(reload);
    await vi.advanceTimersByTimeAsync(3_000);

    expect(reload).not.toHaveBeenCalled();
  });

  it("localizes Unix milliseconds in semantic time elements", () => {
    document.body.innerHTML = `
      <time data-unix-ms="1735689602000">1735689602000 (Unix ms)</time>
    `;

    initializeShell();

    const time = document.querySelector("time")!;
    expect(time.textContent).not.toContain("Unix ms");
    expect(time.dateTime).toBe("2025-01-01T00:00:02.000Z");
  });

  it("requires confirmation before submitting a destructive form", () => {
    document.body.innerHTML = `
      <form data-confirm-message="このルールを終了します。">
        <button type="submit">このルールを終了</button>
      </form>
    `;
    const confirm = vi.fn().mockReturnValue(false);
    Object.defineProperty(window, "confirm", {
      configurable: true,
      value: confirm,
    });
    initializeShell();

    const form = document.querySelector("form")!;
    const event = new SubmitEvent("submit", {
      bubbles: true,
      cancelable: true,
    });
    const accepted = form.dispatchEvent(event);

    expect(confirm).toHaveBeenCalledWith("このルールを終了します。");
    expect(accepted).toBe(false);
  });

  it("opens and focuses the section selected after a save", () => {
    document.body.dataset.focusTarget = "rule-rule_02";
    document.body.innerHTML = `
      <section data-setting-tabs data-default-setting-tab="normal">
        <div role="tablist">
          <button type="button" role="tab" data-setting-tab="normal">
            通常の値
          </button>
          <button type="button" role="tab" data-setting-tab="alarm">
            異常検知
          </button>
        </div>
        <div role="tabpanel" data-setting-panel="normal"></div>
        <div role="tabpanel" data-setting-panel="alarm">
          <details id="rule-rule_02">
            <summary>温度アラームの設定</summary>
          </details>
        </div>
      </section>
    `;
    initializeShell();

    const details = document.querySelector("details")!;
    expect(details.open).toBe(true);
    expect(document.activeElement).toBe(details.querySelector("summary"));
    expect(
      document
        .querySelector('[data-setting-tab="alarm"]')
        ?.getAttribute("aria-selected"),
    ).toBe("true");
    expect(
      document.querySelector<HTMLElement>(
        '[data-setting-panel="alarm"]',
      )?.hidden,
    ).toBe(false);
  });

  it("switches sensor setting panels without leaving the page", () => {
    document.body.innerHTML = `
      <section data-setting-tabs data-default-setting-tab="normal">
        <div role="tablist">
          <button type="button" role="tab" data-setting-tab="basic">
            基本設定
          </button>
          <button type="button" role="tab" data-setting-tab="normal">
            通常の値
          </button>
          <button type="button" role="tab" data-setting-tab="alarm">
            異常検知
          </button>
        </div>
        <div role="tabpanel" data-setting-panel="basic"></div>
        <div role="tabpanel" data-setting-panel="normal"></div>
        <div role="tabpanel" data-setting-panel="alarm"></div>
      </section>
    `;

    initializeShell();

    const normal = document.querySelector<HTMLButtonElement>(
      '[data-setting-tab="normal"]',
    )!;
    const alarm = document.querySelector<HTMLButtonElement>(
      '[data-setting-tab="alarm"]',
    )!;
    expect(normal.getAttribute("aria-selected")).toBe("true");
    expect(
      document.querySelector<HTMLElement>(
        '[data-setting-panel="normal"]',
      )?.hidden,
    ).toBe(false);

    alarm.click();

    expect(normal.getAttribute("aria-selected")).toBe("false");
    expect(alarm.getAttribute("aria-selected")).toBe("true");
    expect(
      document.querySelector<HTMLElement>(
        '[data-setting-panel="normal"]',
      )?.hidden,
    ).toBe(true);
    expect(
      document.querySelector<HTMLElement>(
        '[data-setting-panel="alarm"]',
      )?.hidden,
    ).toBe(false);
  });

  it("replaces the sensor setting tab query without losing unrelated parameters", () => {
    window.history.replaceState(
      { source: "test" },
      "",
      "/equipment/devices/device-01/sensors/signal-01?keep=1&tab=basic&tab=alarm&saved=1#rules",
    );
    const replaceState = vi.spyOn(window.history, "replaceState");
    const pushState = vi.spyOn(window.history, "pushState");
    const historyLength = window.history.length;
    document.body.innerHTML = `
      <section data-setting-tabs data-default-setting-tab="basic">
        <div role="tablist">
          <button type="button" role="tab" data-setting-tab="basic">
            基本設定
          </button>
          <button type="button" role="tab" data-setting-tab="normal">
            計測ルール
          </button>
        </div>
        <div role="tabpanel" data-setting-panel="basic"></div>
        <div role="tabpanel" data-setting-panel="normal"></div>
      </section>
    `;

    initializeShell();
    document
      .querySelector<HTMLButtonElement>('[data-setting-tab="normal"]')!
      .click();

    expect(replaceState).toHaveBeenCalledOnce();
    expect(pushState).not.toHaveBeenCalled();
    expect(window.history.length).toBe(historyLength);
    expect(window.location.pathname).toBe(
      "/equipment/devices/device-01/sensors/signal-01",
    );
    expect(window.location.search).toBe("?keep=1&tab=normal&saved=1");
    expect(window.location.hash).toBe("#rules");
    expect(
      new URLSearchParams(window.location.search).getAll("tab"),
    ).toEqual(["normal"]);
  });
});
