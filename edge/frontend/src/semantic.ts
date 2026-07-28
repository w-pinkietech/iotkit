import type { components } from "./generated/edge-api";
import {
  formField,
  numericFormField,
  query,
  queryAll,
  requiredFormField,
} from "./dom";

export type DefinitionSpec = components["schemas"]["DefinitionSpec"];
export type RuleSpec = components["schemas"]["RuleSpec"];
export type SemanticKind = components["schemas"]["SemanticKind"];
type DetectorMode = NonNullable<
  components["schemas"]["Detector"]["mode"]
>;
type TriggerMode = NonNullable<components["schemas"]["TriggerMode"]>;

const semanticKinds: ReadonlySet<string> = new Set([
  "numeric",
  "boolean",
  "cumulative_counter",
  "alarm",
]);
const detectorModes: ReadonlySet<string> = new Set([
  "",
  "boolean_high_active",
  "boolean_low_active",
  "high_active",
  "low_active",
]);
const triggerModes: ReadonlySet<string> = new Set([
  "",
  "on_transition",
  "on_notification",
]);

function selectedValue<T extends string>(
  value: string,
  allowed: ReadonlySet<string>,
  field: string,
): T {
  if (!allowed.has(value)) {
    throw new Error(`unsupported ${field}: ${value}`);
  }
  return value as T;
}

function detectorSpec(
  form: HTMLFormElement,
): components["schemas"]["Detector"] {
  return {
    mode: selectedValue<DetectorMode>(
      formField(form, "detector_mode")?.value ?? "",
      detectorModes,
      "detector mode",
    ),
    rise_threshold: numericFormField(form, "rise_threshold"),
    fall_threshold: numericFormField(form, "fall_threshold"),
    rise_debounce_ms: Math.round(
      numericFormField(form, "rise_debounce_seconds") * 1000,
    ),
    fall_debounce_ms: Math.round(
      numericFormField(form, "fall_debounce_seconds") * 1000,
    ),
  };
}

function semanticKind(form: HTMLFormElement): SemanticKind {
  return selectedValue(
    requiredFormField(form, "kind").value,
    semanticKinds,
    "semantic kind",
  );
}

function triggerMode(
  form: HTMLFormElement,
): TriggerMode {
  return selectedValue(
    formField(form, "trigger")?.value ?? "",
    triggerModes,
    "trigger mode",
  );
}

export function definitionSpec(form: HTMLFormElement): DefinitionSpec {
  return {
    kind: semanticKind(form),
    scale: numericFormField(form, "scale"),
    offset: numericFormField(form, "offset"),
    detector: detectorSpec(form),
    trigger: triggerMode(form),
  };
}

export function ruleSpec(form: HTMLFormElement): RuleSpec {
  return {
    kind: semanticKind(form),
    detector: detectorSpec(form),
    trigger: triggerMode(form),
  };
}

function toggleFields(
  container: HTMLElement | null,
  visible: boolean,
): void {
  if (!container) return;
  container.hidden = !visible;
  for (const field of queryAll<HTMLInputElement | HTMLSelectElement>(
    "input, select",
    container,
  )) {
    field.disabled = !visible;
  }
}

function initializeSemanticFields(form: HTMLFormElement): void {
  const kind = query<HTMLSelectElement>("[data-semantic-kind]", form);
  const detectorFields = query<HTMLElement>("[data-semantic-detector]", form);
  const detector = formField<HTMLSelectElement>(form, "detector_mode");
  const thresholds = query<HTMLElement>("[data-semantic-thresholds]", form);
  const triggerFields = query<HTMLElement>("[data-semantic-trigger]", form);
  const trigger = formField<HTMLSelectElement>(form, "trigger");
  const booleanInput = form.dataset.booleanInput === "true";
  if (!kind || !detectorFields || !detector || !triggerFields || !trigger) {
    return;
  }

  const update = (): void => {
    const needsDetector = kind.value !== "numeric";
    const countsValues = kind.value === "cumulative_counter";
    toggleFields(detectorFields, needsDetector);
    toggleFields(triggerFields, countsValues);
    for (const option of Array.from(detector.options)) {
      const matchesInput = booleanInput
        ? option.hasAttribute("data-detector-boolean")
        : option.hasAttribute("data-detector-analog");
      option.hidden = !matchesInput;
      option.disabled = !matchesInput;
    }
    const selected = detector.selectedOptions[0];
    if (needsDetector && (!selected || selected.disabled)) {
      detector.value = booleanInput
        ? "boolean_high_active"
        : "high_active";
    } else if (!needsDetector) {
      detector.value = "";
    }
    toggleFields(thresholds, needsDetector && !booleanInput);

    if (
      countsValues &&
      !["on_transition", "on_notification"].includes(trigger.value)
    ) {
      trigger.value = "on_transition";
    } else if (!countsValues) {
      trigger.value = "";
    }
  };

  kind.addEventListener("change", update);
  update();
}

export function initializeSemanticForms(): void {
  for (const form of queryAll<HTMLFormElement>("form.semantic-form")) {
    initializeSemanticFields(form);
  }
  for (const panel of queryAll<HTMLElement>("[data-setting-panel]")) {
    const targets = queryAll<HTMLDetailsElement>(
      "details[data-preview-target]",
      panel,
    );
    const savedCards = targets.filter((target) =>
      target.classList.contains("semantic-rule-card"),
    );
    if (savedCards.length && !targets.some((target) => target.open)) {
      savedCards[0].open = true;
    }
    for (const target of targets) {
      target.addEventListener("toggle", () => {
        if (!target.open) return;
        for (const peer of targets) {
          if (peer !== target) peer.open = false;
        }
      });
    }
  }
}
