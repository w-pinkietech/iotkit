import { afterEach, describe, expect, it, vi } from "vitest";
import { initializeShell } from "./shell";

afterEach(() => {
  document.body.replaceChildren();
  document.body.removeAttribute("data-focus-target");
  vi.restoreAllMocks();
});

describe("console shell", () => {
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
});
