"use strict";
(() => {
  // src/api.ts
  function isRecord(value) {
    return !!value && typeof value === "object";
  }
  function isAPIError(value) {
    if (!isRecord(value) || !("error" in value)) return false;
    const error = value.error;
    return isRecord(error) && "code" in error && typeof error.code === "string" && "message" in error && typeof error.message === "string";
  }
  function isPreviewPoint(value) {
    if (!isRecord(value)) return false;
    return [
      "received_at",
      "input",
      "input_min",
      "input_max",
      "calibrated",
      "calibrated_min",
      "calibrated_max",
      "sample_count"
    ].every((field) => typeof value[field] === "number");
  }
  function isPreviewBody(value) {
    if (!isRecord(value)) return false;
    const kinds = /* @__PURE__ */ new Set([
      "numeric",
      "boolean",
      "cumulative_counter",
      "alarm"
    ]);
    return typeof value.kind === "string" && kinds.has(value.kind) && typeof value.input_count === "number" && typeof value.plot_count === "number" && (value.points === null || Array.isArray(value.points) && value.points.every(isPreviewPoint));
  }
  function isMappingPreviewResponse(value) {
    if (isPreviewBody(value)) return true;
    if (!isRecord(value) || !isRecord(value.calibration)) return false;
    return typeof value.calibration.scale === "number" && typeof value.calibration.offset === "number" && Array.isArray(value.rules) && value.rules.every(
      (rule) => isPreviewBody(rule) && isRecord(rule) && typeof rule.rule_id === "string" && typeof rule.display_name === "string"
    );
  }
  async function createMappingPreview(request, csrfToken2, signal) {
    const response = await fetch("/api/v1/mapping-previews", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-CSRF-Token": csrfToken2
      },
      body: JSON.stringify(request),
      signal
    });
    const payload = await response.json().catch(() => null);
    if (!response.ok) {
      return {
        ok: false,
        status: response.status,
        error: isAPIError(payload) ? payload : null
      };
    }
    if (!isMappingPreviewResponse(payload)) {
      return { ok: false, status: response.status, error: null };
    }
    return { ok: true, value: payload };
  }

  // src/dom.ts
  function query(selector, root = document) {
    return root.querySelector(selector);
  }
  function queryAll(selector, root = document) {
    return Array.from(root.querySelectorAll(selector));
  }
  function formField(form, name) {
    const candidate = form.elements.namedItem(name);
    return candidate instanceof HTMLInputElement || candidate instanceof HTMLSelectElement || candidate instanceof HTMLTextAreaElement ? candidate : null;
  }
  function requiredFormField(form, name) {
    const candidate = formField(form, name);
    if (!candidate) throw new Error(`missing form field: ${name}`);
    return candidate;
  }
  function numericFormField(form, name, fallback = 0) {
    const value = formField(form, name)?.value.trim();
    return value ? Number(value) : fallback;
  }

  // src/shell.ts
  var SETTING_TAB_CHANGE_EVENT = "iotkit:setting-tab-change";
  function csrfToken() {
    const value = document.cookie.split("; ").find((cookie) => cookie.startsWith("iotkit_edge_csrf="));
    return value?.split("=")[1] ?? "";
  }
  function initializeMenu() {
    const menuButton = query(".menu-button");
    const overlay = query(".mobile-overlay");
    const sidebar = query(".sidebar");
    if (!menuButton || !overlay || !sidebar) return;
    const compactLayout = window.matchMedia("(max-width: 960px)");
    const setOpen = (open, restoreFocus = false) => {
      document.body.classList.toggle("menu-open", open);
      menuButton.setAttribute("aria-expanded", String(open));
      overlay.hidden = !open;
      if (open) {
        (query(".side-nav a.active", sidebar) ?? query(".side-nav a", sidebar))?.focus();
      } else if (restoreFocus) {
        menuButton.focus();
      }
    };
    menuButton.addEventListener("click", () => {
      const open = !document.body.classList.contains("menu-open");
      setOpen(open);
    });
    overlay.addEventListener("click", () => setOpen(false, true));
    for (const link of queryAll(".side-nav a", sidebar)) {
      link.addEventListener("click", () => setOpen(false));
    }
    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape") setOpen(false, true);
    });
    compactLayout.addEventListener("change", (event) => {
      if (!event.matches) setOpen(false);
    });
  }
  function initializeTableFilter(tableID) {
    const table = document.getElementById(tableID);
    if (!(table instanceof HTMLTableElement)) return;
    const search = query(
      `[data-table-search="${tableID}"]`
    );
    const status = query(
      `[data-table-status="${tableID}"]`
    );
    const count = query(`[data-table-count="${tableID}"]`);
    const apply = () => {
      const searchText = search?.value.trim().toLocaleLowerCase("ja") ?? "";
      const selectedState = status?.value ?? "";
      let visible = 0;
      for (const row of queryAll(
        "tbody tr:not(.empty-row)",
        table
      )) {
        const rowText = row.textContent?.toLocaleLowerCase("ja") ?? "";
        const matchesText = !searchText || rowText.includes(searchText);
        const matchesState = !selectedState || row.dataset.status === selectedState;
        row.hidden = !(matchesText && matchesState);
        if (!row.hidden) visible += 1;
      }
      if (count) count.textContent = String(visible);
    };
    search?.addEventListener("input", apply);
    status?.addEventListener("change", apply);
  }
  function initializeDocumentActions() {
    for (const form of queryAll("form[data-confirm-message]")) {
      form.addEventListener("submit", (event) => {
        const message = form.dataset.confirmMessage;
        if (message && !window.confirm(message)) {
          event.preventDefault();
        }
      });
    }
    document.addEventListener("click", (event) => {
      if (!(event.target instanceof Element)) return;
      const copyButton = event.target.closest("[data-copy-text]");
      if (copyButton) {
        const originalLabel = copyButton.textContent;
        navigator.clipboard.writeText(copyButton.dataset.copyText ?? "").then(() => {
          copyButton.textContent = "\u30B3\u30D4\u30FC\u3057\u307E\u3057\u305F";
        }).catch(() => {
          copyButton.textContent = "\u30B3\u30D4\u30FC\u3067\u304D\u307E\u305B\u3093";
        }).finally(() => {
          window.setTimeout(() => {
            copyButton.textContent = originalLabel;
          }, 1600);
        });
        return;
      }
      const row = event.target.closest("tr[data-href]");
      if (!row || event.target.closest("a, button, input, select, textarea") || !row.dataset.href) {
        return;
      }
      window.location.assign(row.dataset.href);
    });
  }
  function initializeSettingTabs() {
    for (const root of queryAll("[data-setting-tabs]")) {
      const tabs = queryAll("[data-setting-tab]", root);
      const panels = queryAll("[data-setting-panel]", root);
      if (!tabs.length || !panels.length) continue;
      const activate = (key, focus = false, replaceTabQuery = false) => {
        for (const tab of tabs) {
          const selected = tab.dataset.settingTab === key;
          tab.setAttribute("aria-selected", String(selected));
          tab.tabIndex = selected ? 0 : -1;
          if (selected && focus) tab.focus();
        }
        for (const panel of panels) {
          panel.hidden = panel.dataset.settingPanel !== key;
        }
        if (replaceTabQuery) {
          const url = new URL(window.location.href);
          url.searchParams.set("tab", key);
          window.history.replaceState(window.history.state, "", url);
        }
        root.dispatchEvent(
          new CustomEvent(SETTING_TAB_CHANGE_EVENT, {
            detail: { key },
            bubbles: true
          })
        );
      };
      let initial = root.dataset.defaultSettingTab ?? tabs[0].dataset.settingTab;
      const focusedID = document.body.dataset.focusTarget;
      const focused = focusedID ? document.getElementById(focusedID) : null;
      const focusedPanel = focused && root.contains(focused) ? focused.closest("[data-setting-panel]") : null;
      if (focusedPanel?.dataset.settingPanel) {
        initial = focusedPanel.dataset.settingPanel;
      }
      activate(initial ?? "");
      root.classList.add("setting-tabs-ready");
      tabs.forEach((tab, index) => {
        tab.addEventListener("click", () => {
          activate(tab.dataset.settingTab ?? "", false, true);
        });
        tab.addEventListener("keydown", (event) => {
          let next = index;
          if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
          else if (event.key === "ArrowLeft") {
            next = (index - 1 + tabs.length) % tabs.length;
          } else if (event.key === "Home") next = 0;
          else if (event.key === "End") next = tabs.length - 1;
          else return;
          event.preventDefault();
          activate(tabs[next].dataset.settingTab ?? "", true, true);
        });
      });
    }
  }
  function initializeFocusedSection() {
    const targetID = document.body.dataset.focusTarget;
    if (!targetID) return;
    const target = document.getElementById(targetID);
    if (!target) return;
    if (target instanceof HTMLDetailsElement) {
      target.open = true;
      target.querySelector("summary")?.focus();
      return;
    }
    target.setAttribute("tabindex", "-1");
    target.focus();
  }
  function initializeLocalizedTimes() {
    const timeZone = document.body.dataset.displayTimeZone || "UTC";
    const formatter = new Intl.DateTimeFormat("ja-JP", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
      timeZone,
      timeZoneName: "short"
    });
    for (const timestamp of queryAll("[data-unix-ms]")) {
      const milliseconds = Number(timestamp.dataset.unixMs);
      if (!Number.isFinite(milliseconds)) continue;
      const date = new Date(milliseconds);
      if (Number.isNaN(date.getTime())) continue;
      if (timestamp instanceof HTMLTimeElement) {
        timestamp.dateTime = date.toISOString();
      }
      timestamp.textContent = formatter.format(date);
    }
    const checkedAt = query("[data-activation-checked-at]");
    if (checkedAt) {
      const now = /* @__PURE__ */ new Date();
      checkedAt.dateTime = now.toISOString();
      checkedAt.textContent = now.toLocaleTimeString("ja-JP", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit"
      });
    }
  }
  function initializeActivationRefresh(reload) {
    const key = `iotkit-activation-refresh:${window.location.pathname}`;
    if (document.body.dataset.activationRefresh !== "true") {
      sessionStorage.removeItem(key);
      return;
    }
    const checkNow = query("[data-activation-check-now]");
    if (checkNow && checkNow.dataset.activationBound !== "true") {
      checkNow.dataset.activationBound = "true";
      checkNow.addEventListener("click", () => {
        sessionStorage.setItem(key, "0");
        reload();
      });
    }
    const attempts = Number(sessionStorage.getItem(key) ?? "0");
    if (!Number.isFinite(attempts) || attempts >= 20) {
      const state = query("[data-activation-state]");
      const guidance = query("[data-activation-guidance]");
      if (state) state.textContent = "\u81EA\u52D5\u78BA\u8A8D\u3092\u4E00\u6642\u505C\u6B62\u3057\u307E\u3057\u305F";
      if (guidance) {
        guidance.textContent = "\u81EA\u52D5\u78BA\u8A8D\u306E\u4E0A\u9650\u306B\u9054\u3057\u305F\u305F\u3081\u4E00\u6642\u505C\u6B62\u3057\u307E\u3057\u305F\u3002\u30B5\u30FC\u30D0\u30FC\u5074\u306E\u767B\u9332\u51E6\u7406\u306F\u7D9A\u3044\u3066\u3044\u307E\u3059\u3002\u300C\u4ECA\u3059\u3050\u78BA\u8A8D\u300D\u3067\u78BA\u8A8D\u3092\u518D\u958B\u3067\u304D\u307E\u3059\u3002";
      }
      return;
    }
    window.setTimeout(() => {
      sessionStorage.setItem(key, String(attempts + 1));
      reload();
    }, 3e3);
  }
  function initializeSignalProfile(form) {
    const sensorType = query("[data-sensor-type]", form);
    const customLabel = query("[data-custom-sensor-label]", form);
    const valueKind = query("[data-value-kind]", form);
    const unitMode = query("[data-unit-mode]", form);
    const unitField = query("[data-display-unit]", form);
    const decimalField = query("[data-decimal-places]", form);
    const update = () => {
      if (customLabel) {
        const usesCustomLabel = sensorType?.value === "custom";
        customLabel.hidden = !usesCustomLabel;
        const input = query("input", customLabel);
        if (input) input.required = usesCustomLabel;
      }
      if (valueKind?.value === "boolean") {
        if (unitMode) unitMode.value = "dimensionless";
        const unitInput2 = unitField ? query("input", unitField) : null;
        if (unitInput2) unitInput2.value = "";
        const decimalInput = decimalField ? query("input", decimalField) : null;
        if (decimalInput) decimalInput.value = "0";
      }
      const hasUnit = unitMode?.value === "unit" && valueKind?.value !== "boolean";
      if (unitField) unitField.hidden = !hasUnit;
      const unitInput = unitField ? query("input", unitField) : null;
      if (unitInput) {
        unitInput.required = hasUnit;
        unitInput.disabled = !hasUnit;
        if (!hasUnit) unitInput.value = "";
      }
      if (decimalField) decimalField.hidden = valueKind?.value === "boolean";
    };
    sensorType?.addEventListener("change", update);
    valueKind?.addEventListener("change", update);
    unitMode?.addEventListener("change", update);
    update();
  }
  function initializeShell(reload = () => window.location.reload()) {
    initializeMenu();
    initializeTableFilter("signal-table");
    initializeTableFilter("log-table");
    initializeDocumentActions();
    initializeLocalizedTimes();
    initializeActivationRefresh(reload);
    initializeSettingTabs();
    for (const form of queryAll("form[data-signal-profile]")) {
      initializeSignalProfile(form);
    }
    initializeFocusedSection();
  }

  // src/semantic.ts
  var semanticKinds = /* @__PURE__ */ new Set([
    "numeric",
    "boolean",
    "cumulative_counter",
    "alarm"
  ]);
  var detectorModes = /* @__PURE__ */ new Set([
    "",
    "boolean_high_active",
    "boolean_low_active",
    "high_active",
    "low_active"
  ]);
  var triggerModes = /* @__PURE__ */ new Set([
    "",
    "on_transition",
    "on_notification"
  ]);
  function selectedValue(value, allowed, field) {
    if (!allowed.has(value)) {
      throw new Error(`unsupported ${field}: ${value}`);
    }
    return value;
  }
  function detectorSpec(form) {
    return {
      mode: selectedValue(
        formField(form, "detector_mode")?.value ?? "",
        detectorModes,
        "detector mode"
      ),
      rise_threshold: numericFormField(form, "rise_threshold"),
      fall_threshold: numericFormField(form, "fall_threshold"),
      rise_debounce_ms: Math.round(
        numericFormField(form, "rise_debounce_seconds") * 1e3
      ),
      fall_debounce_ms: Math.round(
        numericFormField(form, "fall_debounce_seconds") * 1e3
      )
    };
  }
  function semanticKind(form) {
    return selectedValue(
      requiredFormField(form, "kind").value,
      semanticKinds,
      "semantic kind"
    );
  }
  function triggerMode(form) {
    return selectedValue(
      formField(form, "trigger")?.value ?? "",
      triggerModes,
      "trigger mode"
    );
  }
  function definitionSpec(form) {
    return {
      kind: semanticKind(form),
      scale: numericFormField(form, "scale"),
      offset: numericFormField(form, "offset"),
      detector: detectorSpec(form),
      trigger: triggerMode(form)
    };
  }
  function ruleSpec(form) {
    return {
      kind: semanticKind(form),
      detector: detectorSpec(form),
      trigger: triggerMode(form)
    };
  }
  function toggleFields(container, visible) {
    if (!container) return;
    container.hidden = !visible;
    for (const field of queryAll(
      "input, select",
      container
    )) {
      field.disabled = !visible;
    }
  }
  function initializeSemanticFields(form) {
    const kind = query("[data-semantic-kind]", form);
    const detectorFields = query("[data-semantic-detector]", form);
    const detector = formField(form, "detector_mode");
    const thresholds = query("[data-semantic-thresholds]", form);
    const triggerFields = query("[data-semantic-trigger]", form);
    const trigger = formField(form, "trigger");
    const booleanInput = form.dataset.booleanInput === "true";
    if (!kind || !detectorFields || !detector || !triggerFields || !trigger) {
      return;
    }
    const update = () => {
      const needsDetector = kind.value !== "numeric";
      const countsValues = kind.value === "cumulative_counter";
      toggleFields(detectorFields, needsDetector);
      toggleFields(triggerFields, countsValues);
      for (const option of Array.from(detector.options)) {
        const matchesInput = booleanInput ? option.hasAttribute("data-detector-boolean") : option.hasAttribute("data-detector-analog");
        option.hidden = !matchesInput;
        option.disabled = !matchesInput;
      }
      const selected = detector.selectedOptions[0];
      if (needsDetector && (!selected || selected.disabled)) {
        detector.value = booleanInput ? "boolean_high_active" : "high_active";
      } else if (!needsDetector) {
        detector.value = "";
      }
      toggleFields(thresholds, needsDetector && !booleanInput);
      if (countsValues && !["on_transition", "on_notification"].includes(trigger.value)) {
        trigger.value = "on_transition";
      } else if (!countsValues) {
        trigger.value = "";
      }
    };
    kind.addEventListener("change", update);
    update();
  }
  function initializeSemanticForms() {
    for (const form of queryAll("form.semantic-form")) {
      initializeSemanticFields(form);
    }
    for (const panel of queryAll("[data-setting-panel]")) {
      const targets = queryAll(
        "details[data-preview-target]",
        panel
      );
      const savedCards = targets.filter(
        (target) => target.classList.contains("semantic-rule-card")
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

  // src/preview.ts
  var kindLabels = {
    numeric: "\u6E2C\u5B9A\u5024",
    boolean: "ON / OFF",
    cumulative_counter: "\u7D2F\u7A4D\u5024",
    alarm: "\u7570\u5E38\u691C\u77E5"
  };
  var svgNamespace = "http://www.w3.org/2000/svg";
  function addSVG(parent, name, attributes = {}) {
    const element = document.createElementNS(svgNamespace, name);
    for (const [key, value] of Object.entries(attributes)) {
      element.setAttribute(key, String(value));
    }
    parent.appendChild(element);
    return element;
  }
  function isFiniteNumber(value) {
    return Number.isFinite(Number(value));
  }
  function formatNumber(value) {
    return Number(value).toLocaleString("ja-JP", {
      maximumFractionDigits: 3
    });
  }
  function formatCurrentValue(value, valueKind, decimalPlaces) {
    if (valueKind === "boolean") {
      return Number(value) === 0 ? "OFF" : "ON";
    }
    if (!Number.isInteger(decimalPlaces)) return formatNumber(value);
    const digits = Math.min(6, Math.max(0, Number(decimalPlaces)));
    return Number(value).toLocaleString("ja-JP", {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits
    });
  }
  function formatDuration(start, end) {
    const milliseconds = Math.max(0, end - start);
    if (milliseconds < 6e4) {
      return `${Math.max(1, Math.round(milliseconds / 1e3))}\u79D2`;
    }
    return `${Math.max(1, Math.round(milliseconds / 6e4))}\u5206`;
  }
  function setText(element, value) {
    if (element.textContent !== value) element.textContent = value;
  }
  function clearFieldErrors(panel) {
    for (const error of queryAll(".field-error", panel)) {
      error.remove();
    }
    for (const field of queryAll('[aria-invalid="true"]', panel)) {
      field.removeAttribute("aria-invalid");
      const describedBy = (field.getAttribute("aria-describedby") ?? "").split(/\s+/).filter((id) => id && !id.startsWith("preview-field-error-"));
      if (describedBy.length) {
        field.setAttribute("aria-describedby", describedBy.join(" "));
      } else {
        field.removeAttribute("aria-describedby");
      }
    }
  }
  function showFieldError(field, label) {
    field.setAttribute("aria-invalid", "true");
    const wrapper = field.closest("label");
    if (!wrapper) return;
    const error = document.createElement("small");
    error.className = "field-error";
    error.id = `preview-field-error-${field.getAttribute("name") ?? "field"}`;
    error.textContent = `${label}\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044\u3002`;
    wrapper.append(error);
    const describedBy = new Set(
      (field.getAttribute("aria-describedby") ?? "").split(/\s+/).filter(Boolean)
    );
    describedBy.add(error.id);
    field.setAttribute("aria-describedby", Array.from(describedBy).join(" "));
  }
  function kindLabel(kind) {
    return kindLabels[kind];
  }
  function latestRuleOutcome(payload, unit) {
    const latest = payload.points?.at(-1);
    if (!latest) {
      return {
        value: "\u53D7\u4FE1\u5F85\u3061",
        detail: "\u53D7\u4FE1\u30C7\u30FC\u30BF\u3092\u5F85\u3063\u3066\u3044\u307E\u3059\u3002",
        alarm: false
      };
    }
    switch (payload.kind) {
      case "boolean":
        return {
          value: latest.active ? "ON" : "OFF",
          detail: "\u73FE\u5728\u306E\u5224\u5B9A",
          alarm: false
        };
      case "cumulative_counter":
        return {
          value: `\u7D2F\u7A4D ${formatNumber(latest.counter ?? 0)}`,
          detail: Number(latest.increment ?? 0) > 0 ? `\u4ECA\u56DE +${formatNumber(latest.increment)}` : "\u4ECA\u56DE\u306E\u5897\u5206\u306A\u3057",
          alarm: false
        };
      case "alarm":
        return {
          value: latest.active ? "\u7570\u5E38" : "\u6B63\u5E38",
          detail: latest.active ? "\u7570\u5E38\u6761\u4EF6\u306B\u8A72\u5F53" : "\u6B63\u5E38\u7BC4\u56F2",
          alarm: Boolean(latest.active)
        };
      default:
        return {
          value: `${formatNumber(latest.calibrated)}${unit ? ` ${unit}` : ""}`,
          detail: "\u88DC\u6B63\u5F8C\u306E\u5024",
          alarm: false
        };
    }
  }
  function renderRuleResult(panel, selected, state, unit) {
    const container = query("[data-preview-rule-result]", panel);
    const name = query("[data-preview-rule-name]", panel);
    const kind = query("[data-preview-rule-kind]", panel);
    const value = query("[data-preview-rule-value]", panel);
    const detail = query("[data-preview-rule-detail]", panel);
    if (!container || !name || !kind || !value || !detail) return null;
    container.classList.remove("is-alarm");
    if (state === "error" && selected?.error) {
      setText(
        name,
        `${selected.display_name}\uFF08\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\uFF09`
      );
      setText(kind, kindLabel(selected.kind));
      setText(value, "\u2014");
      setText(detail, "\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002");
      return null;
    }
    if (state !== "ready" || !selected) {
      const messages = {
        none: [
          "\u9078\u629E\u4E2D\u306E\u30EB\u30FC\u30EB\u306F\u3042\u308A\u307E\u305B\u3093",
          "\u2014",
          "\u30EB\u30FC\u30EB\u3092\u958B\u304F\u3068\u5224\u5B9A\u7D50\u679C\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
        ],
        invalid: [
          "\u8A2D\u5B9A\u5185\u5BB9\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044",
          "\u2014",
          "\u5165\u529B\u9805\u76EE\u3092\u4FEE\u6B63\u3057\u3066\u304F\u3060\u3055\u3044\u3002"
        ],
        error: [
          "\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093",
          "\u2014",
          "\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
        ]
      };
      const [title, result, hint] = messages[state === "ready" ? "none" : state];
      setText(name, title);
      setText(kind, "\u2014");
      setText(value, result);
      setText(detail, hint);
      return null;
    }
    const outcome = latestRuleOutcome(selected, unit);
    setText(name, selected.display_name);
    setText(kind, kindLabel(selected.kind));
    setText(value, outcome.value);
    setText(detail, outcome.detail);
    container.classList.toggle("is-alarm", outcome.alarm);
    return outcome;
  }
  function clearAuxiliaryOutputs(summary, testResult, state) {
    const messages = {
      none: "\u30B0\u30E9\u30D5\u306B\u8868\u793A\u3067\u304D\u308B\u53D7\u4FE1\u30C7\u30FC\u30BF\u306F\u307E\u3060\u3042\u308A\u307E\u305B\u3093\u3002",
      invalid: "\u8A2D\u5B9A\u5185\u5BB9\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002",
      error: "\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
    };
    if (summary) setText(summary, messages[state]);
    if (testResult) {
      setText(testResult, "\u5024\u3092\u5165\u529B\u3059\u308B\u3068\u7D50\u679C\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059");
    }
  }
  function updateAccessibleSummary(summary, raw, selected, outcome, unit) {
    if (!summary) return;
    const points = raw.points ?? [];
    if (selected?.error) {
      if (!points.length) {
        setText(
          summary,
          `\u53D7\u4FE1\u5024\u306F\u307E\u3060\u3042\u308A\u307E\u305B\u3093\u3002\u9078\u629E\u4E2D\u306F${selected.display_name}\u3001${kindLabel(selected.kind)}\u3067\u3059\u304C\u3001\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002`
        );
        return;
      }
      const inputs2 = points.flatMap((point) => [
        Number(point.input_min),
        Number(point.input_max)
      ]);
      const count2 = raw.input_count ?? points.length;
      setText(
        summary,
        `\u53D7\u4FE1\u5024\u306F${formatNumber(Math.min(...inputs2))}\u304B\u3089${formatNumber(Math.max(...inputs2))}\u3067\u3059\u3002\u9078\u629E\u4E2D\u306F${selected.display_name}\u3001${kindLabel(selected.kind)}\u3067\u3059\u304C\u3001\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002${count2}\u4EF6\u306E\u53D7\u4FE1\u30C7\u30FC\u30BF\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002`
      );
      return;
    }
    if (!points.length) {
      setText(
        summary,
        selected ? `${selected.display_name}\u306F\u53D7\u4FE1\u30C7\u30FC\u30BF\u3092\u5F85\u3063\u3066\u3044\u307E\u3059\u3002` : "\u30B0\u30E9\u30D5\u306B\u8868\u793A\u3067\u304D\u308B\u53D7\u4FE1\u30C7\u30FC\u30BF\u306F\u307E\u3060\u3042\u308A\u307E\u305B\u3093\u3002"
      );
      return;
    }
    const inputs = points.flatMap((point) => [
      Number(point.input_min),
      Number(point.input_max)
    ]);
    const count = raw.input_count ?? points.length;
    const ruleText = selected && outcome ? `\u9078\u629E\u4E2D\u306F${selected.display_name}\u3001${kindLabel(selected.kind)}\u3001\u73FE\u5728\u306F${outcome.value}\u3067\u3059\u3002` : "\u9078\u629E\u4E2D\u306E\u30EB\u30FC\u30EB\u306F\u3042\u308A\u307E\u305B\u3093\u3002";
    const calibratedText = selected?.kind === "numeric" && outcome ? (() => {
      const calibrated = points.flatMap((point) => [
        Number(point.calibrated_min),
        Number(point.calibrated_max)
      ]);
      const latest = points.at(-1);
      if (!latest || !calibrated.length) return "";
      return `\u88DC\u6B63\u5F8C\u306F${formatNumber(Math.min(...calibrated))}\u304B\u3089${formatNumber(Math.max(...calibrated))}\u3001\u6700\u65B0\u306E\u88DC\u6B63\u5F8C\u306F${formatNumber(latest.calibrated)}${unit ? ` ${unit}` : ""}\u3067\u3059\u3002`;
    })() : "";
    setText(
      summary,
      `\u53D7\u4FE1\u5024\u306F${formatNumber(Math.min(...inputs))}\u304B\u3089${formatNumber(Math.max(...inputs))}\u3067\u3059\u3002${calibratedText}${ruleText}${count}\u4EF6\u306E\u53D7\u4FE1\u30C7\u30FC\u30BF\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002`
    );
  }
  function previewWindow(payload, points) {
    const now = Date.now();
    return {
      start: payload.window_start ?? points[0]?.received_at ?? now,
      end: payload.window_end ?? points.at(-1)?.received_at ?? now
    };
  }
  function renderEmptyChart(svg, payload) {
    const title = addSVG(svg, "text", {
      x: 380,
      y: 122,
      "text-anchor": "middle",
      class: "chart-empty-title"
    });
    title.textContent = payload.error ? "\u3053\u306E\u30EB\u30FC\u30EB\u3067\u306F\u53D7\u4FE1\u5024\u3092\u5224\u5B9A\u3067\u304D\u307E\u305B\u3093" : "\u307E\u3060\u53D7\u4FE1\u30C7\u30FC\u30BF\u304C\u3042\u308A\u307E\u305B\u3093";
    const hint = addSVG(svg, "text", {
      x: 380,
      y: 148,
      "text-anchor": "middle",
      class: "chart-empty-hint"
    });
    hint.textContent = payload.error ? "\u5165\u529B\u5024\u306E\u88DC\u6B63\u3068\u5224\u5B9A\u6761\u4EF6\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044" : "\u8A66\u3059\u5024\u3092\u5165\u529B\u3057\u3066\u3001\u8A2D\u5B9A\u7D50\u679C\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059";
  }
  function renderPreviewChart(svg, payload, showSemanticOverlays) {
    svg.replaceChildren();
    const points = payload.points ?? [];
    const width = 760;
    const height = 260;
    const left = 58;
    const right = 18;
    const top = 18;
    const bottom = 42;
    const plotWidth = width - left - right;
    const plotHeight = height - top - bottom;
    if (!points.length) {
      renderEmptyChart(svg, payload);
      return;
    }
    const values = [];
    for (const point of points) {
      const pointValues = showSemanticOverlays ? [
        point.input_min,
        point.input_max,
        point.calibrated_min,
        point.calibrated_max
      ] : [point.input_min, point.input_max];
      for (const value of pointValues) {
        if (isFiniteNumber(value)) values.push(Number(value));
      }
    }
    if (showSemanticOverlays && isFiniteNumber(payload.rise_threshold)) {
      values.push(payload.rise_threshold);
    }
    if (showSemanticOverlays && isFiniteNumber(payload.fall_threshold)) {
      values.push(payload.fall_threshold);
    }
    let minValue = Math.min(...values);
    let maxValue = Math.max(...values);
    if (minValue === maxValue) {
      const padding = Math.max(1, Math.abs(minValue) * 0.1);
      minValue -= padding;
      maxValue += padding;
    } else {
      const padding = (maxValue - minValue) * 0.08;
      minValue -= padding;
      maxValue += padding;
    }
    const firstReceivedAt = points[0].received_at;
    const lastReceivedAt = points.at(-1)?.received_at ?? firstReceivedAt;
    const x = (index) => {
      if (points.length === 1) return left + plotWidth / 2;
      const point = points[index];
      if (lastReceivedAt > firstReceivedAt) {
        return left + (point.received_at - firstReceivedAt) * plotWidth / (lastReceivedAt - firstReceivedAt);
      }
      return left + index * plotWidth / (points.length - 1);
    };
    const y = (value) => top + (maxValue - value) * plotHeight / (maxValue - minValue);
    for (let index = 0; index <= 4; index += 1) {
      const gridY = top + index * plotHeight / 4;
      addSVG(svg, "line", {
        x1: left,
        x2: width - right,
        y1: gridY,
        y2: gridY,
        class: "chart-grid"
      });
      const label = addSVG(svg, "text", {
        x: left - 9,
        y: gridY + 4,
        "text-anchor": "end",
        class: "chart-axis-label"
      });
      label.textContent = formatNumber(
        maxValue - index * (maxValue - minValue) / 4
      );
    }
    const drawThreshold = (value, labelText) => {
      if (!isFiniteNumber(value)) return;
      const thresholdY = y(value);
      addSVG(svg, "line", {
        x1: left,
        x2: width - right,
        y1: thresholdY,
        y2: thresholdY,
        class: "chart-threshold"
      });
      const label = addSVG(svg, "text", {
        x: width - right - 4,
        y: thresholdY - 6,
        "text-anchor": "end",
        class: "chart-threshold-label"
      });
      label.textContent = `${labelText} ${formatNumber(value)}`;
    };
    if (showSemanticOverlays) {
      drawThreshold(payload.rise_threshold, "\u7ACB\u4E0A\u308A");
      drawThreshold(payload.fall_threshold, "\u7ACB\u4E0B\u308A");
    }
    points.forEach((point, index) => {
      if (point.sample_count > 1) {
        addSVG(svg, "line", {
          x1: x(index),
          x2: x(index),
          y1: y(point.input_min),
          y2: y(point.input_max),
          class: "chart-range"
        });
        if (showSemanticOverlays) {
          addSVG(svg, "line", {
            x1: x(index) + 2,
            x2: x(index) + 2,
            y1: y(point.calibrated_min),
            y2: y(point.calibrated_max),
            class: "chart-range-result"
          });
        }
      }
      if (showSemanticOverlays && payload.kind !== "numeric") {
        const ratio = point.sample_count ? Number(point.active_samples ?? 0) / point.sample_count : 0;
        if (ratio > 0) {
          addSVG(svg, "rect", {
            x: x(index) - Math.max(1, plotWidth / Math.max(points.length, 1) / 2),
            y: top,
            width: Math.max(2, plotWidth / Math.max(points.length, 1)),
            height: plotHeight,
            class: "chart-active-band",
            opacity: Math.max(0.12, ratio * 0.24)
          });
        }
      }
    });
    const path = (field) => points.map(
      (point, index) => `${index === 0 ? "M" : "L"} ${x(index).toFixed(2)} ${y(point[field]).toFixed(2)}`
    ).join(" ");
    addSVG(svg, "path", {
      d: path("input"),
      class: "chart-line chart-line-raw"
    });
    if (showSemanticOverlays) {
      addSVG(svg, "path", {
        d: path("calibrated"),
        class: "chart-line chart-line-result"
      });
    }
    const latestPoint = points.at(-1);
    if (showSemanticOverlays && latestPoint) {
      addSVG(svg, "circle", {
        cx: x(points.length - 1),
        cy: y(latestPoint.calibrated),
        r: 5,
        class: "chart-latest-point"
      });
      const latestLabel = addSVG(svg, "text", {
        x: Math.min(width - right - 4, x(points.length - 1) - 8),
        y: Math.max(top + 13, y(latestPoint.calibrated) - 10),
        "text-anchor": "end",
        class: "chart-latest-label"
      });
      latestLabel.textContent = "\u6700\u65B0";
    }
    if (showSemanticOverlays && payload.kind === "cumulative_counter") {
      const maxIncrement = Math.max(
        1,
        ...points.map((point) => Number(point.increment ?? 0))
      );
      points.forEach((point, index) => {
        const increment = Number(point.increment ?? 0);
        if (!increment) return;
        const barHeight = Math.max(3, increment / maxIncrement * 34);
        addSVG(svg, "rect", {
          x: x(index) - 2,
          y: top + plotHeight - barHeight,
          width: 4,
          height: barHeight,
          class: "chart-increment"
        });
      });
      const maxCounter = Math.max(
        1,
        ...points.map((point) => Number(point.counter ?? 0))
      );
      const counterY = (value) => top + (maxCounter - Number(value ?? 0)) * plotHeight / maxCounter;
      const counterPath = points.map(
        (point, index) => `${index === 0 ? "M" : "L"} ${x(index).toFixed(2)} ${counterY(point.counter).toFixed(2)}`
      ).join(" ");
      addSVG(svg, "path", {
        d: counterPath,
        class: "chart-line chart-line-counter"
      });
      const latestCounter = points.at(-1)?.counter;
      const counterLabel = addSVG(svg, "text", {
        x: width - right - 4,
        y: counterY(latestCounter) - 7,
        "text-anchor": "end",
        class: "chart-counter-label"
      });
      counterLabel.textContent = `\u7D2F\u7A4D ${formatNumber(latestCounter ?? 0)}`;
    }
    const window2 = previewWindow(payload, points);
    const start = addSVG(svg, "text", {
      x: left,
      y: height - 14,
      class: "chart-axis-label"
    });
    start.textContent = new Date(window2.start).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit"
    });
    const end = addSVG(svg, "text", {
      x: width - right,
      y: height - 14,
      "text-anchor": "end",
      class: "chart-axis-label"
    });
    end.textContent = new Date(window2.end).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit"
    });
  }
  function isMultipleRulePreview(response) {
    return "rules" in response;
  }
  function activePreviewID(scope) {
    const activePanel = queryAll(
      "[data-setting-panel]",
      scope
    ).find((panel) => !panel.hidden);
    const form = activePanel?.querySelector(
      "details[data-preview-target][open] form.semantic-form[data-preview-id]"
    );
    return form?.dataset.previewId;
  }
  function selectPreview(response, activeID) {
    if (!isMultipleRulePreview(response)) {
      return { raw: response, selected: null };
    }
    const withWindow = (rule) => rule ? {
      ...rule,
      window_start: response.window_start,
      window_end: response.window_end,
      truncated_by: response.truncated_by
    } : null;
    return {
      raw: withWindow(
        response.rules.find((rule) => !rule.error) ?? response.rules[0]
      ),
      selected: withWindow(
        activeID ? response.rules.find((rule) => rule.rule_id === activeID) : void 0
      )
    };
  }
  function rawOnlyPreview(payload) {
    return {
      ...payload,
      kind: "numeric",
      test_result: void 0,
      rise_threshold: void 0,
      fall_threshold: void 0,
      points: (payload.points ?? []).map((point) => ({
        ...point,
        calibrated: point.input,
        calibrated_min: point.input_min,
        calibrated_max: point.input_max,
        active: void 0,
        active_samples: void 0,
        transitions: void 0,
        counter: void 0,
        increment: void 0
      }))
    };
  }
  function buildRequest(signalRef, forms, calibrationForm, multipleRules, testInput, activeID) {
    const body = { signal_ref: signalRef };
    const firstForm = forms[0];
    if (multipleRules) {
      const rules = forms.filter((candidate) => {
        const previewID = candidate.dataset.previewId;
        const hasName = !!formField(candidate, "display_name")?.value.trim();
        return !!previewID && (hasName || previewID === activeID);
      }).map((candidate) => ({
        rule_id: candidate.dataset.previewId,
        display_name: formField(candidate, "display_name")?.value.trim() || (candidate.dataset.previewId === "draft-alarm" ? "\u65B0\u3057\u3044\u7570\u5E38\u691C\u77E5" : "\u65B0\u3057\u3044\u8A08\u6E2C\u30EB\u30FC\u30EB"),
        spec: ruleSpec(candidate)
      }));
      if (!rules.length) {
        rules.push({
          rule_id: "draft-raw",
          display_name: "\u53D7\u4FE1\u5024",
          spec: { kind: "numeric" }
        });
      }
      if (rules.length) {
        body.calibration = {
          scale: calibrationForm ? numericFormField(calibrationForm, "scale") : 1,
          offset: calibrationForm ? numericFormField(calibrationForm, "offset") : 0
        };
        body.rules = rules;
      }
    } else if (firstForm) {
      body.spec = definitionSpec(firstForm);
    }
    const testValue = testInput?.value.trim();
    if (testValue) body.test_value = Number(testValue);
    return body;
  }
  function initializePreview(panel) {
    const signalRef = panel.dataset.signalRef;
    if (!signalRef) return;
    const previewScope = panel.closest(".sensor-setting-workspace") ?? document.body;
    const forms = queryAll(
      `form.semantic-form[data-signal-ref="${signalRef}"]`
    );
    const calibrationForm = query(
      `form[action="/console/signals/${signalRef}/calibration"]`
    );
    const multipleRules = forms.some((form) => !!form.dataset.ruleId) || forms.some((form) => form.action.endsWith("/semantic-rules"));
    const testInput = query(
      '[name="preview_test_value"]',
      panel
    );
    const testResult = query("[data-preview-test-result]", panel);
    const range = query("[data-preview-range]", panel);
    const count = query("[data-preview-count]", panel);
    const message = query("[data-preview-message]", panel);
    const feedState = query("[data-preview-feed-state]", panel);
    const checkedAt = query("[data-preview-checked-at]", panel);
    const accessibleSummary = query(
      "[data-preview-accessible-summary]",
      panel
    );
    const toggle = query("[data-preview-toggle]", panel);
    const chart = query("[data-preview-chart]", panel);
    const resultLegend = query(
      "[data-preview-result-legend]",
      panel
    );
    const thresholdLegend = query(
      "[data-preview-threshold-legend]",
      panel
    );
    const currentValue = query(
      "[data-preview-current-value]",
      panel
    );
    const currentReceived = query(
      "[data-preview-current-received]",
      panel
    );
    const unit = panel.dataset.unit ?? "";
    if (!range || !count || !message || !chart) return;
    const sourceSummary = query(
      ".sensor-detail-latest[data-source-value]"
    );
    const sourceCurrentValue = sourceSummary ? query("[data-source-current-value]", sourceSummary) : null;
    const sourceCurrentReceived = sourceSummary ? query("[data-source-current-received]", sourceSummary) : null;
    const valueKind = query(
      'form[data-signal-profile] [name="display_value_kind"]'
    );
    const decimalPlaces = query(
      'form[data-signal-profile] [name="decimal_places"]'
    );
    let controller;
    let debounce;
    let previewUnavailable = false;
    let paused = false;
    let lastSeenReceivedAt;
    const setSemanticLegends = (visible, payload) => {
      if (resultLegend) resultLegend.hidden = !visible;
      if (thresholdLegend) {
        thresholdLegend.hidden = !visible || !payload || !isFiniteNumber(payload.rise_threshold) && !isFiniteNumber(payload.fall_threshold);
      }
    };
    const setFeedState = (state) => {
      if (feedState) setText(feedState, state);
    };
    const markChecked = () => {
      if (!checkedAt) return;
      setText(
        checkedAt,
        `\u78BA\u8A8D ${(/* @__PURE__ */ new Date()).toLocaleTimeString("ja-JP", {
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit"
        })}`
      );
    };
    const refresh = async () => {
      controller?.abort();
      controller = new AbortController();
      clearFieldErrors(previewScope);
      const activeID = activePreviewID(previewScope);
      setSemanticLegends(false);
      const body = buildRequest(
        signalRef,
        forms,
        calibrationForm,
        multipleRules,
        testInput,
        activeID
      );
      try {
        const result = await createMappingPreview(
          body,
          csrfToken(),
          controller.signal
        );
        if (!result.ok) {
          const fieldName = result.error?.error.field;
          const activeForm = forms.find(
            (candidate) => candidate.dataset.previewId === activeID
          );
          const invalidField = fieldName && activeForm ? formField(activeForm, fieldName) : null;
          const fieldLabel = invalidField?.closest("label")?.querySelector(":scope > span")?.textContent?.trim();
          if (result.status === 404 && !forms[0]) {
            previewUnavailable = true;
            renderRuleResult(panel, null, "none", unit);
            clearAuxiliaryOutputs(accessibleSummary, testResult, "none");
            setFeedState("\u8868\u793A\u3059\u308B\u30EB\u30FC\u30EB\u304C\u3042\u308A\u307E\u305B\u3093");
            setText(
              message,
              "\u5024\u306E\u5909\u63DB\u304C\u8A2D\u5B9A\u3055\u308C\u308B\u3068\u3001\u3053\u3053\u306B\u8A2D\u5B9A\u7D50\u679C\u3092\u8868\u793A\u3057\u307E\u3059\u3002"
            );
          } else if (fieldLabel && invalidField) {
            renderRuleResult(panel, null, "invalid", unit);
            clearAuxiliaryOutputs(accessibleSummary, testResult, "invalid");
            setFeedState("\u8A2D\u5B9A\u5185\u5BB9\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044");
            showFieldError(invalidField, fieldLabel);
            setText(
              message,
              `${fieldLabel}\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044\u3002\u6700\u5F8C\u306B\u78BA\u8A8D\u3067\u304D\u305F\u30B0\u30E9\u30D5\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002`
            );
          } else {
            renderRuleResult(
              panel,
              null,
              result.status === 400 ? "invalid" : "error",
              unit
            );
            clearAuxiliaryOutputs(
              accessibleSummary,
              testResult,
              result.status === 400 ? "invalid" : "error"
            );
            setFeedState("\u66F4\u65B0\u3092\u78BA\u8A8D\u3067\u304D\u307E\u305B\u3093");
            setText(
              message,
              "\u8A2D\u5B9A\u5185\u5BB9\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044\u3002\u6700\u5F8C\u306B\u78BA\u8A8D\u3067\u304D\u305F\u30B0\u30E9\u30D5\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002"
            );
          }
          return;
        }
        const selection = selectPreview(result.value, activeID);
        const selectedReady = selection.selected && !selection.selected.error ? selection.selected : null;
        const selectedFailure = selection.selected?.error ? selection.selected : null;
        const payload = selectedReady ?? (selection.raw ? rawOnlyPreview(selection.raw) : null);
        if (!payload) {
          renderRuleResult(
            panel,
            null,
            activeID ? "error" : "none",
            unit
          );
          clearAuxiliaryOutputs(
            accessibleSummary,
            testResult,
            activeID ? "error" : "none"
          );
          setFeedState("\u8868\u793A\u3059\u308B\u30EB\u30FC\u30EB\u304C\u3042\u308A\u307E\u305B\u3093");
          setText(message, "\u78BA\u8A8D\u3067\u304D\u308B\u30EB\u30FC\u30EB\u304C\u3042\u308A\u307E\u305B\u3093\u3002");
          return;
        }
        setSemanticLegends(Boolean(selectedReady), payload);
        renderPreviewChart(chart, payload, Boolean(selectedReady));
        const resultState = !activeID ? "none" : selectedReady ? "ready" : "error";
        const outcome = renderRuleResult(
          panel,
          selectedReady ?? selectedFailure,
          resultState,
          unit
        );
        updateAccessibleSummary(
          accessibleSummary,
          payload,
          selectedReady ?? selectedFailure,
          outcome,
          unit
        );
        const points = payload.points ?? [];
        const latest = selection.raw?.points?.at(-1);
        markChecked();
        if (!latest) {
          setFeedState("\u53D7\u4FE1\u5F85\u3061");
        } else if (lastSeenReceivedAt === void 0) {
          setFeedState("\u5B9F\u30C7\u30FC\u30BF\u3092\u8868\u793A\u4E2D");
          lastSeenReceivedAt = latest.received_at;
        } else if (latest.received_at > lastSeenReceivedAt) {
          setFeedState("\u65B0\u3057\u3044\u30C7\u30FC\u30BF\u3092\u53D7\u4FE1");
          lastSeenReceivedAt = latest.received_at;
        } else {
          setFeedState("\u65B0\u7740\u306A\u3057");
        }
        if (latest && currentValue) {
          currentValue.textContent = formatCurrentValue(
            latest.input,
            valueKind?.value,
            decimalPlaces ? Number(decimalPlaces.value) : void 0
          );
        }
        if (latest && sourceCurrentValue && sourceSummary) {
          const rawValue = formatCurrentValue(
            latest.input,
            valueKind?.value,
            decimalPlaces ? Number(decimalPlaces.value) : void 0
          );
          sourceCurrentValue.textContent = rawValue;
          sourceSummary.dataset.sourceValue = rawValue;
        }
        if (latest && (currentReceived || sourceCurrentReceived)) {
          const elapsed = Math.max(0, Date.now() - latest.received_at);
          const relative = elapsed < 5e3 ? "\u305F\u3063\u305F\u4ECA" : elapsed < 6e4 ? `${Math.floor(elapsed / 1e3)}\u79D2\u524D` : `${Math.floor(elapsed / 6e4)}\u5206\u524D`;
          const receivedTitle = new Date(latest.received_at).toLocaleString(
            "ja-JP"
          );
          if (currentReceived) {
            currentReceived.textContent = `\u6700\u7D42\u53D7\u4FE1 ${relative}`;
            currentReceived.title = receivedTitle;
          }
          if (sourceCurrentReceived) {
            sourceCurrentReceived.textContent = relative;
            sourceCurrentReceived.title = receivedTitle;
          }
        }
        if (payload.input_count === 0) {
          range.textContent = "\u53D7\u4FE1\u30C7\u30FC\u30BF\u306F\u307E\u3060\u3042\u308A\u307E\u305B\u3093";
          count.textContent = "\u8A66\u3059\u5024\u3067\u8A2D\u5B9A\u7D50\u679C\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002";
          setText(message, "\u5C65\u6B74\u306F\u4F5C\u3089\u305A\u3001\u5B9F\u969B\u306B\u5C4A\u3044\u305F\u5024\u3060\u3051\u3092\u8868\u793A\u3057\u307E\u3059\u3002");
        } else {
          const window2 = previewWindow(payload, points);
          range.textContent = `\u76F4\u8FD1${formatDuration(window2.start, window2.end)}\u306E\u53D7\u4FE1\u5024`;
          count.textContent = `${payload.input_count.toLocaleString("ja-JP")}\u4EF6\u3092${payload.plot_count.toLocaleString("ja-JP")}\u70B9\u3067\u8868\u793A`;
          const valuesOverlap = points.every(
            (point) => Math.abs(point.input - point.calibrated) < 1e-9
          );
          setText(
            message,
            payload.truncated_by === "input_count" ? "\u9AD8\u901F\u306A\u4FE1\u53F7\u306E\u305F\u3081\u3001\u6700\u65B020,000\u4EF6\u3092\u8981\u7D04\u3057\u3066\u3044\u307E\u3059\u3002" : payload.kind === "cumulative_counter" ? `\u8868\u793A\u7BC4\u56F2\u5185\u306E\u7D2F\u7A4D\u5024\u306F ${points.at(-1)?.counter ?? 0} \u3067\u3059\u3002\u5148\u982D\u306E\u5024\u306F\u6570\u3048\u307E\u305B\u3093\u3002` : valuesOverlap ? "\u5909\u63DB\u524D\u5F8C\u306E\u5024\u306F\u540C\u3058\u3067\u3059\u3002\u88DC\u6B63\u3092\u5909\u66F4\u3059\u308B\u3068\u5DEE\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002" : "\u8A2D\u5B9A\u3092\u5909\u3048\u308B\u3068\u3001\u4FDD\u5B58\u524D\u306E\u7D50\u679C\u3092\u3053\u306E\u30B0\u30E9\u30D5\u3067\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
          );
        }
        if (selectedFailure) {
          setText(
            message,
            "\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
          );
        }
        if (testResult) {
          const previewResult = payload.test_result;
          if (!previewResult) {
            testResult.textContent = "\u5024\u3092\u5165\u529B\u3059\u308B\u3068\u7D50\u679C\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059";
          } else {
            switch (payload.kind) {
              case "boolean":
                testResult.textContent = previewResult.boolean ? "ON" : "OFF";
                break;
              case "alarm":
                testResult.textContent = previewResult.boolean ? "\u7570\u5E38" : "\u6B63\u5E38";
                break;
              case "cumulative_counter":
                testResult.textContent = previewResult.integer !== void 0 ? `\u7D2F\u7A4D ${formatNumber(previewResult.integer)}` : "\u6700\u521D\u306E\u5024\u3068\u3057\u3066\u78BA\u8A8D\uFF08\u7D2F\u7A4D\u306B\u306F\u52A0\u3048\u307E\u305B\u3093\uFF09";
                break;
              default:
                if (previewResult.number !== void 0) {
                  testResult.textContent = `${formatNumber(previewResult.number)}${unit ? ` ${unit}` : ""}`;
                } else {
                  testResult.textContent = `\u88DC\u6B63\u5F8C ${formatNumber(previewResult.calibrated)}${unit ? ` ${unit}` : ""}`;
                }
                break;
            }
          }
        }
      } catch (error) {
        if (!(error instanceof DOMException && error.name === "AbortError")) {
          renderRuleResult(panel, null, "error", unit);
          clearAuxiliaryOutputs(accessibleSummary, testResult, "error");
          setFeedState("\u66F4\u65B0\u3092\u78BA\u8A8D\u3067\u304D\u307E\u305B\u3093");
          setText(
            message,
            "\u8A2D\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u30C7\u30FC\u30BF\u53D7\u4FE1\u306B\u306F\u5F71\u97FF\u3042\u308A\u307E\u305B\u3093\u3002"
          );
        }
      }
    };
    const schedule = () => {
      if (debounce !== void 0) window.clearTimeout(debounce);
      debounce = window.setTimeout(refresh, 300);
    };
    for (const form of forms) {
      form.addEventListener("input", schedule);
      form.addEventListener("change", schedule);
    }
    calibrationForm?.addEventListener("input", schedule);
    calibrationForm?.addEventListener("change", schedule);
    previewScope.addEventListener(SETTING_TAB_CHANGE_EVENT, schedule);
    for (const target of queryAll(
      "details[data-preview-target]",
      previewScope
    )) {
      target.addEventListener("toggle", schedule);
    }
    testInput?.addEventListener("input", schedule);
    toggle?.addEventListener("click", () => {
      paused = !paused;
      toggle.setAttribute("aria-checked", String(!paused));
      const state = query("[data-preview-toggle-state]", toggle);
      if (state) state.textContent = paused ? "OFF" : "ON";
      panel.classList.toggle("preview-paused", paused);
      if (paused) {
        setFeedState("\u66F4\u65B0\u505C\u6B62\u4E2D");
      } else {
        setFeedState("\u53D7\u4FE1\u30C7\u30FC\u30BF\u3092\u78BA\u8A8D\u4E2D");
        void refresh();
      }
    });
    void refresh();
    window.setInterval(() => {
      if (document.visibilityState === "visible" && !previewUnavailable && !paused) {
        void refresh();
      }
    }, 1e3);
  }
  function initializePreviews() {
    for (const panel of queryAll("[data-setting-simulation]")) {
      initializePreview(panel);
    }
  }

  // src/console.ts
  initializeShell();
  initializeSemanticForms();
  initializePreviews();
})();
