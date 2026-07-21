import { afterEach, describe, expect, it } from "vitest";
import { initializeSemanticForms } from "./semantic";

afterEach(() => {
  document.body.replaceChildren();
});

describe("semantic form behavior", () => {
  it("keeps the rule selected by the save return path open", () => {
    document.body.innerHTML = `
      <details class="semantic-rule-card"></details>
      <details class="semantic-rule-card" open></details>
    `;

    initializeSemanticForms();

    const cards = Array.from(
      document.querySelectorAll<HTMLDetailsElement>(".semantic-rule-card"),
    );
    expect(cards[0].open).toBe(false);
    expect(cards[1].open).toBe(true);
  });

  it("shows only fields that affect the selected result kind", () => {
    document.body.innerHTML = `
      <form class="semantic-form" data-boolean-input="false">
        <select name="kind" data-semantic-kind>
          <option value="numeric" selected>numeric</option>
          <option value="cumulative_counter">counter</option>
          <option value="alarm">alarm</option>
        </select>
        <div data-semantic-detector>
          <select name="detector_mode">
            <option value="boolean_high_active" data-detector-boolean>boolean</option>
            <option value="high_active" data-detector-analog>analog</option>
          </select>
          <div data-semantic-thresholds>
            <input name="rise_threshold" value="1">
            <input name="fall_threshold" value="0">
          </div>
        </div>
        <div data-semantic-trigger>
          <select name="trigger">
            <option value="on_transition">transition</option>
            <option value="on_notification">sample</option>
          </select>
        </div>
      </form>
    `;

    initializeSemanticForms();

    const kind = document.querySelector<HTMLSelectElement>('[name="kind"]')!;
    const detector = document.querySelector<HTMLElement>(
      "[data-semantic-detector]",
    )!;
    const detectorMode = document.querySelector<HTMLSelectElement>(
      '[name="detector_mode"]',
    )!;
    const thresholds = document.querySelector<HTMLElement>(
      "[data-semantic-thresholds]",
    )!;
    const triggerFields = document.querySelector<HTMLElement>(
      "[data-semantic-trigger]",
    )!;
    const trigger = document.querySelector<HTMLSelectElement>(
      '[name="trigger"]',
    )!;

    expect(detector.hidden).toBe(true);
    expect(triggerFields.hidden).toBe(true);
    expect(detectorMode.value).toBe("");
    expect(trigger.value).toBe("");

    kind.value = "cumulative_counter";
    kind.dispatchEvent(new Event("change"));

    expect(detector.hidden).toBe(false);
    expect(thresholds.hidden).toBe(false);
    expect(triggerFields.hidden).toBe(false);
    expect(detectorMode.value).toBe("high_active");
    expect(trigger.value).toBe("on_transition");

    kind.value = "alarm";
    kind.dispatchEvent(new Event("change"));

    expect(detector.hidden).toBe(false);
    expect(triggerFields.hidden).toBe(true);
    expect(trigger.value).toBe("");
  });
});
