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
  function isHistorySeries(value) {
    return isRecord(value) && typeof value.signal_ref === "string" && (typeof value.latest_received_at === "number" || value.latest_received_at === null) && Array.isArray(value.points) && value.points.every(
      (point) => isRecord(point) && typeof point.bucket_start === "number" && typeof point.minimum === "number" && typeof point.average === "number" && typeof point.maximum === "number" && (!("last_value" in point) || typeof point.last_value === "number") && typeof point.sample_count === "number"
    );
  }
  async function getHistorySeries(ruleId, from, to, bucketMs, signal) {
    const query2 = new URLSearchParams({
      rule_id: ruleId,
      from: String(from),
      to: String(to),
      bucket_ms: String(bucketMs)
    });
    const response = await fetch(`/api/v1/history/series?${query2}`, { signal });
    const payload = await response.json().catch(() => null);
    if (!response.ok) {
      return {
        ok: false,
        status: response.status,
        error: isAPIError(payload) ? payload : null
      };
    }
    return isHistorySeries(payload) ? { ok: true, value: payload } : { ok: false, status: response.status, error: null };
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

  // src/signal-chart.ts
  var SVG_NS = "http://www.w3.org/2000/svg";
  var GEOMETRIES = {
    preview: {
      width: 760,
      height: 260,
      left: 58,
      right: 18,
      top: 18,
      bottom: 42
    },
    compact: {
      width: 360,
      height: 160,
      left: 72,
      right: 12,
      top: 12,
      bottom: 28
    }
  };
  function addSVG(parent, name, attributes = {}) {
    const element = document.createElementNS(SVG_NS, name);
    for (const [key, value] of Object.entries(attributes)) {
      element.setAttribute(key, String(value));
    }
    parent.append(element);
    return element;
  }
  function finite(value) {
    return typeof value === "number" && Number.isFinite(value);
  }
  function numberLabel(value) {
    return value.toLocaleString("ja-JP", { maximumFractionDigits: 3 });
  }
  function drawText(svg, text, attributes) {
    const element = addSVG(svg, "text", attributes);
    element.textContent = text;
  }
  function emptyChart(svg, title, hint, geometry) {
    drawText(svg, title, {
      x: geometry.width / 2,
      y: geometry.height / 2 - 8,
      "text-anchor": "middle",
      class: "chart-empty-title live-chart-empty"
    });
    drawText(svg, hint, {
      x: geometry.width / 2,
      y: geometry.height / 2 + 18,
      "text-anchor": "middle",
      class: "chart-empty-hint live-chart-empty-hint"
    });
  }
  function pathFor(points, x, y, value, step) {
    let path = "";
    let previous;
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
  function sameShape(target, source) {
    return target.tagName === source.tagName && target.getAttribute("class") === source.getAttribute("class");
  }
  function syncElement(target, source, syncAttributes = true) {
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
  function syncChartDOM(target, source) {
    target.setAttribute("viewBox", source.getAttribute("viewBox") ?? "");
    const geometry = source.dataset.chartGeometry;
    if (geometry) target.dataset.chartGeometry = geometry;
    syncElement(target, source, false);
  }
  function renderSignalChartDraft(svg, options) {
    const geometry = GEOMETRIES[options.geometry ?? "preview"];
    const plotWidth = geometry.width - geometry.left - geometry.right;
    const plotHeight = geometry.height - geometry.top - geometry.bottom;
    svg.setAttribute(
      "viewBox",
      `0 0 ${geometry.width} ${geometry.height}`
    );
    svg.dataset.chartGeometry = options.geometry ?? "preview";
    const points = options.points.filter(
      (point) => finite(point.at) && finite(point.value)
    );
    if (!points.length) {
      emptyChart(
        svg,
        options.emptyTitle ?? "\u307E\u3060\u53D7\u4FE1\u30C7\u30FC\u30BF\u304C\u3042\u308A\u307E\u305B\u3093",
        options.emptyHint ?? "\u5B9F\u969B\u306B\u5C4A\u3044\u305F\u5024\u3092\u5F85\u3063\u3066\u3044\u307E\u3059",
        geometry
      );
      return 0;
    }
    const startAt = finite(options.startAt) ? options.startAt : points[0].at;
    const latestPointAt = points.at(-1)?.at ?? startAt;
    const requestedEnd = finite(options.endAt) ? options.endAt : latestPointAt;
    const endAt = Math.max(startAt + 1e3, requestedEnd);
    const x = (at) => geometry.left + Math.max(0, Math.min(1, (at - startAt) / (endAt - startAt))) * plotWidth;
    const rawValues = points.flatMap((point) => [
      finite(point.minimum) ? point.minimum : point.value,
      finite(point.maximum) ? point.maximum : point.value
    ]);
    const resultValues = options.showResult ? points.flatMap(
      (point) => [point.resultMinimum, point.resultMaximum, point.result].filter(finite)
    ) : [];
    const thresholdValues = options.thresholds ? [options.thresholds.rise, options.thresholds.fall].filter(finite) : [];
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
    const y = (value) => geometry.top + (maximum - value) * plotHeight / (maximum - minimum);
    for (let index = 0; index <= 4; index += 1) {
      const gridY = geometry.top + index * plotHeight / 4;
      addSVG(svg, "line", {
        x1: geometry.left,
        x2: geometry.width - geometry.right,
        y1: gridY,
        y2: gridY,
        class: "chart-grid"
      });
      drawText(
        svg,
        options.boolean ? index === 0 ? "ON" : index === 4 ? "OFF" : "" : numberLabel(maximum - index * (maximum - minimum) / 4),
        {
          x: geometry.left - 9,
          y: gridY + 4,
          "text-anchor": "end",
          class: "chart-axis-label"
        }
      );
    }
    const drawThreshold = (value, labelText) => {
      if (!finite(value)) return;
      const thresholdY = y(value);
      addSVG(svg, "line", {
        x1: geometry.left,
        x2: geometry.width - geometry.right,
        y1: thresholdY,
        y2: thresholdY,
        class: "chart-threshold"
      });
      drawText(svg, `${labelText} ${numberLabel(value)}`, {
        x: geometry.width - geometry.right - 4,
        y: thresholdY - 6,
        "text-anchor": "end",
        class: "chart-threshold-label"
      });
    };
    drawThreshold(options.thresholds?.rise, "\u7ACB\u4E0A\u308A");
    drawThreshold(options.thresholds?.fall, "\u7ACB\u4E0B\u308A");
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
          class: "chart-range"
        });
        if (options.showResult) {
          const resultMinimum = finite(point.resultMinimum) ? point.resultMinimum : point.result;
          const resultMaximum = finite(point.resultMaximum) ? point.resultMaximum : point.result;
          if (finite(resultMinimum) && finite(resultMaximum)) {
            addSVG(svg, "line", {
              x1: x(point.at) + 2,
              x2: x(point.at) + 2,
              y1: y(resultMinimum),
              y2: y(resultMaximum),
              class: "chart-range-result"
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
          opacity: Math.max(0.12, ratio * 0.24)
        });
      });
    }
    const rawPath = pathFor(
      points,
      x,
      y,
      (point) => options.boolean ? point.value >= 0.5 ? 1 : 0 : point.value,
      options.rawStep ?? Boolean(options.boolean)
    );
    if (rawPath) {
      addSVG(svg, "path", {
        d: rawPath,
        class: "chart-line chart-line-raw live-chart-line"
      });
    }
    if (options.showResult) {
      const resultPath = pathFor(
        points,
        x,
        y,
        (point) => point.result,
        Boolean(options.resultStep)
      );
      if (resultPath) {
        addSVG(svg, "path", {
          d: resultPath,
          class: "chart-line chart-line-result live-chart-result-line"
        });
      }
    }
    const latestPoint = points.at(-1);
    if (options.showLatestMarker && latestPoint) {
      const latestAt = finite(options.latestAt) ? options.latestAt : latestPoint.at;
      const latestValue = options.showResult && finite(latestPoint.result) ? latestPoint.result : options.boolean ? latestPoint.value >= 0.5 ? 1 : 0 : latestPoint.value;
      const latestX = x(latestAt);
      const latestY = y(latestValue);
      addSVG(svg, "line", {
        x1: latestX,
        x2: latestX,
        y1: latestY,
        y2: geometry.top + plotHeight,
        class: "chart-latest-guide live-chart-latest-guide"
      });
      addSVG(svg, "circle", {
        cx: latestX,
        cy: latestY,
        r: 5,
        class: "chart-latest-point live-chart-latest-point"
      });
      drawText(svg, "\u6700\u65B0", {
        x: Math.min(geometry.width - geometry.right - 4, latestX - 8),
        y: Math.max(geometry.top + 13, latestY - 10),
        "text-anchor": "end",
        class: "chart-latest-label live-chart-latest-label"
      });
    }
    const startLabel = options.axisLabels?.start ?? new Date(startAt).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit"
    });
    const endLabel = options.axisLabels?.end ?? new Date(endAt).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit"
    });
    drawText(svg, startLabel, {
      x: geometry.left,
      y: geometry.height - 14,
      class: "chart-axis-label"
    });
    drawText(svg, endLabel, {
      x: geometry.width - geometry.right,
      y: geometry.height - 14,
      "text-anchor": "end",
      class: "chart-axis-label"
    });
    const title = addSVG(svg, "title");
    title.textContent = options.title ?? `\u6A2A\u8EF8\u306F\u76F4\u8FD1\u306E\u6642\u9593\u3001\u7E26\u8EF8\u306F\u5024${options.unit ? `\uFF08${options.unit}\uFF09` : ""}\u3067\u3059\u3002`;
    return points.length;
  }
  function renderSignalChart(svg, options) {
    const draft = document.createElementNS(SVG_NS, "svg");
    const plottedCount = renderSignalChartDraft(draft, options);
    syncChartDOM(svg, draft);
    return plottedCount;
  }

  // src/preview.ts
  var COUNTER_WINDOW_MS = 6e4;
  var COUNTER_BUCKET_MS = 1e3;
  var COUNTER_SESSION_MAX_POINTS = 1e3;
  var PREVIEW_SESSION_MAX_POINTS = 1e3;
  var kindLabels = {
    numeric: "\u6E2C\u5B9A\u5024",
    boolean: "ON / OFF",
    cumulative_counter: "\u7D2F\u7A4D\u5024",
    alarm: "\u7570\u5E38\u691C\u77E5"
  };
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
  function pointPlotAt(point) {
    return isFiniteNumber(point.plot_at) ? Number(point.plot_at) : point.received_at;
  }
  function rawPreviewPoint(point) {
    return {
      ...point,
      calibrated: point.input,
      calibrated_min: point.input_min,
      calibrated_max: point.input_max,
      active: void 0,
      active_samples: void 0,
      transitions: void 0,
      counter: void 0,
      increment: void 0
    };
  }
  function sampleWeight(point) {
    return Math.max(1, Number(point.sample_count) || 0);
  }
  function mergePreviewPoints(left, right) {
    const leftWeight = sampleWeight(left);
    const rightWeight = sampleWeight(right);
    const sampleCount = leftWeight + rightWeight;
    const hasActiveSamples = left.active_samples !== void 0 || right.active_samples !== void 0;
    const hasTransitions = left.transitions !== void 0 || right.transitions !== void 0;
    const hasIncrement = left.increment !== void 0 || right.increment !== void 0;
    return {
      ...right,
      input: (left.input * leftWeight + right.input * rightWeight) / sampleCount,
      input_min: Math.min(left.input_min, right.input_min),
      input_max: Math.max(left.input_max, right.input_max),
      calibrated: (left.calibrated * leftWeight + right.calibrated * rightWeight) / sampleCount,
      calibrated_min: Math.min(left.calibrated_min, right.calibrated_min),
      calibrated_max: Math.max(left.calibrated_max, right.calibrated_max),
      sample_count: sampleCount,
      active_samples: hasActiveSamples ? (left.active_samples ?? 0) + (right.active_samples ?? 0) : void 0,
      transitions: hasTransitions ? (left.transitions ?? 0) + (right.transitions ?? 0) : void 0,
      increment: hasIncrement ? (left.increment ?? 0) + (right.increment ?? 0) : void 0
    };
  }
  function compactAdjacent(points, maximum, weight, merge) {
    const compacted = [...points];
    if (maximum < 2) return compacted.slice(0, maximum);
    while (compacted.length > maximum) {
      let best = 1;
      let bestWeight = weight(compacted[1]) + weight(compacted[2]);
      for (let index = 2; index < compacted.length - 1; index += 1) {
        const combined = weight(compacted[index]) + weight(compacted[index + 1]);
        if (combined < bestWeight) {
          best = index;
          bestWeight = combined;
        }
      }
      compacted.splice(best, 2, merge(compacted[best], compacted[best + 1]));
    }
    return compacted;
  }
  function pointAt(point) {
    return pointPlotAt(point.raw);
  }
  function mergePreviewSessionPoints(left, right) {
    return {
      raw: mergePreviewPoints(left.raw, right.raw),
      result: left.result && right.result ? mergePreviewPoints(left.result, right.result) : void 0
    };
  }
  function sortedSessionPoints(points) {
    const byAt = /* @__PURE__ */ new Map();
    for (const point of points) byAt.set(pointAt(point), point);
    return [...byAt.values()].sort((left, right) => pointAt(left) - pointAt(right));
  }
  function retainFullerSessionPoint(cached, incoming) {
    return {
      raw: cached.raw,
      // Results can only stay paired with this raw aggregate when both came
      // from the current result identity. Result invalidation removes cached
      // results before a changed identity reaches this path.
      result: cached.result && incoming.result ? cached.result : void 0
    };
  }
  function cachedSessionPoints(cache) {
    const byAt = /* @__PURE__ */ new Map();
    for (const point of [...cache.archive, ...cache.recent]) {
      const cached = byAt.get(pointAt(point));
      byAt.set(
        pointAt(point),
        cached && sampleWeight(point.raw) < sampleWeight(cached.raw) ? retainFullerSessionPoint(cached, point) : point
      );
    }
    return [...byAt.values()].sort((left, right) => pointAt(left) - pointAt(right));
  }
  function mergePreviewPointCache(cache, response) {
    const cached = cachedSessionPoints(cache);
    const cachedByAt = new Map(cached.map((point) => [pointAt(point), point]));
    const recent = sortedSessionPoints(response).map((incoming) => {
      const previous = cachedByAt.get(pointAt(incoming));
      return previous && sampleWeight(incoming.raw) < sampleWeight(previous.raw) ? retainFullerSessionPoint(previous, incoming) : incoming;
    });
    const replaced = new Set(recent.map(pointAt));
    const archiveLimit = PREVIEW_SESSION_MAX_POINTS - recent.length;
    const archive = archiveLimit > 0 ? compactAdjacent(
      cached.filter((point) => !replaced.has(pointAt(point))),
      archiveLimit,
      (point) => sampleWeight(point.raw),
      mergePreviewSessionPoints
    ) : [];
    return { archive, recent };
  }
  function mergePreviewSession(session, rawPoints, resultPoints, resultKey) {
    const results = new Map(
      (resultPoints ?? []).map((point) => [pointPlotAt(point), point])
    );
    const response = rawPoints.filter((point) => pointPlotAt(point) + COUNTER_BUCKET_MS > session.startedAt).map((point) => ({
      raw: rawPreviewPoint(point),
      result: resultKey ? results.get(pointPlotAt(point)) : void 0
    }));
    session.points = mergePreviewPointCache(session.points, response);
    if (resultKey) session.resultKey = resultKey;
  }
  function invalidatePreviewResults(session) {
    session.points = {
      archive: session.points.archive.map(({ raw }) => ({ raw })),
      recent: session.points.recent.map(({ raw }) => ({ raw }))
    };
    session.resultKey = void 0;
  }
  function previewSessionPoints(session, resultKey) {
    return [...session.points.archive, ...session.points.recent].sort((left, right) => pointAt(left) - pointAt(right)).map(({ raw, result }) => result && session.resultKey === resultKey ? {
      ...raw,
      calibrated: result.calibrated,
      calibrated_min: result.calibrated_min,
      calibrated_max: result.calibrated_max,
      active: result.active,
      active_samples: result.active_samples,
      transitions: result.transitions,
      counter: result.counter,
      increment: result.increment
    } : raw);
  }
  function latestPreviewPoint(payload) {
    return payload.latest_point ?? payload.points?.at(-1) ?? void 0;
  }
  function hasMeaningfulResult(points) {
    const epsilon = 1e-9;
    return points.some(
      (point) => Math.abs(point.calibrated - point.input) > epsilon || Math.abs(point.calibrated_min - point.input_min) > epsilon || Math.abs(point.calibrated_max - point.input_max) > epsilon
    );
  }
  function counterWindowDelta(payload) {
    const points = payload.points ?? [];
    const window2 = previewWindow(payload, points);
    return points.reduce((total, point) => {
      const at = pointPlotAt(point);
      if (at + COUNTER_BUCKET_MS <= window2.start || at > window2.end) {
        return total;
      }
      return total + Math.max(0, Number(point.increment ?? 0));
    }, 0);
  }
  function persistedCounterRuleID(forms, activeID, selected) {
    if (!selected || selected.kind !== "cumulative_counter" || !activeID) {
      return void 0;
    }
    return counterRuleIDForActiveForm(forms, activeID);
  }
  function counterRuleIDForActiveForm(forms, activeID) {
    const form = forms.find((candidate) => candidate.dataset.previewId === activeID);
    return form?.dataset.ruleId && formField(form, "kind")?.value === "cumulative_counter" ? form.dataset.ruleId : void 0;
  }
  async function loadCounterHistory(ruleID, signal, end) {
    try {
      const result = await getHistorySeries(
        ruleID,
        end - COUNTER_WINDOW_MS,
        end,
        COUNTER_BUCKET_MS,
        signal
      );
      return result.ok ? { status: "available", value: result.value } : { status: "unavailable" };
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        throw error;
      }
      return { status: "unavailable" };
    }
  }
  function availableCounterHistory(state) {
    return state.history?.status === "available" ? state.history.value : void 0;
  }
  function latestHistoryValue(history) {
    return history && typeof history.latest_value === "number" && Number.isFinite(history.latest_value) ? Number(history.latest_value) : void 0;
  }
  function latestHistoryReceivedAt(history) {
    return typeof history.latest_received_at === "number" && Number.isFinite(history.latest_received_at) ? history.latest_received_at : void 0;
  }
  function retainLatestHistory(previous, next) {
    if (latestHistoryValue(next) !== void 0 || previous === void 0 || latestHistoryValue(previous) === void 0) {
      return next;
    }
    return {
      ...next,
      latest_received_at: previous.latest_received_at,
      latest_value: previous.latest_value
    };
  }
  function compactCounterSessionPoints(points) {
    return compactAdjacent(
      points,
      COUNTER_SESSION_MAX_POINTS,
      (point) => Math.max(1, point.sampleCount),
      (left, right) => ({
        ...right,
        minimum: Math.min(left.minimum, right.minimum),
        maximum: Math.max(left.maximum, right.maximum),
        sampleCount: Math.max(1, left.sampleCount) + Math.max(1, right.sampleCount)
      })
    );
  }
  function appendCounterSessionPoint(points, point) {
    const previous = points.at(-1);
    if (previous === void 0) return [point];
    if (point.at < previous.at) return points;
    if (point.at === previous.at) {
      const replaced = [...points.slice(0, -1), point];
      return replaced.length > 1 && replaced.at(-2)?.value === point.value ? replaced.slice(0, -1) : replaced;
    }
    if (point.value === previous.value) return points;
    return compactCounterSessionPoints([...points, point]);
  }
  function counterCurrentPointAt(session, latestReceivedAt, capturedAt) {
    const previousAt = session.points.at(-1)?.at;
    const minimumAt = previousAt === void 0 ? session.startedAt : previousAt + 1;
    if (latestReceivedAt !== void 0 && latestReceivedAt >= minimumAt) {
      return latestReceivedAt;
    }
    return Math.max(
      Number.isFinite(capturedAt) ? capturedAt : minimumAt,
      minimumAt
    );
  }
  function mergeCounterHistorySession(session, history, capturedAt) {
    let baselineCaptured = session.baselineCaptured;
    let points = session.points;
    const latestValue = latestHistoryValue(history);
    const latestReceivedAt = latestHistoryReceivedAt(history);
    if (!baselineCaptured) {
      baselineCaptured = true;
      if (latestValue !== void 0) {
        const at = latestReceivedAt !== void 0 && latestReceivedAt >= session.startedAt ? counterCurrentPointAt(session, latestReceivedAt, capturedAt) : session.startedAt;
        points = appendCounterSessionPoint(points, {
          at,
          value: latestValue,
          minimum: latestValue,
          maximum: latestValue,
          sampleCount: 1
        });
      }
    }
    if (baselineCaptured && latestValue !== void 0 && points.at(-1)?.value !== latestValue) {
      const at = counterCurrentPointAt(
        { ...session, points },
        latestReceivedAt,
        capturedAt
      );
      points = appendCounterSessionPoint(points, {
        at,
        value: latestValue,
        minimum: latestValue,
        maximum: latestValue,
        sampleCount: 1
      });
    }
    return { ...session, baselineCaptured, points };
  }
  function renderCounterHistoryChart(svg, state, now, sharedWindow) {
    if (state.history?.status !== "available") {
      const unavailable = state.history?.status === "unavailable";
      return renderSignalChart(svg, {
        points: [],
        geometry: "compact",
        axisLabels: { start: "\u8868\u793A\u958B\u59CB", end: "\u73FE\u5728" },
        emptyTitle: unavailable ? "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5C65\u6B74\u3092\u53D6\u5F97\u3067\u304D\u307E\u305B\u3093" : "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5C65\u6B74\u3092\u8AAD\u307F\u8FBC\u3093\u3067\u3044\u307E\u3059",
        emptyHint: unavailable ? "\u63A5\u7D9A\u3092\u78BA\u8A8D\u3057\u3066\u3001\u3082\u3046\u4E00\u5EA6\u8868\u793A\u3057\u3066\u304F\u3060\u3055\u3044" : "\u4FDD\u5B58\u6E08\u307F\u306E\u7D50\u679C\u3092\u78BA\u8A8D\u3057\u3066\u3044\u307E\u3059",
        title: "\u6A2A\u8EF8\u306F\u8868\u793A\u958B\u59CB\u5F8C\u306E\u6642\u9593\u3001\u7E26\u8EF8\u306F\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5024\u3067\u3059\u3002\u8868\u793A\u958B\u59CB\u5F8C\u306E\u5168\u671F\u9593\u3092\u6700\u59271,000\u70B9\u3067\u8868\u793A\u3057\u307E\u3059\u3002"
      });
    }
    const sessionPoints2 = state.session?.points ?? [];
    const lastPoint = sessionPoints2.at(-1);
    const currentAt = lastPoint ? Math.max(lastPoint.at, sharedWindow?.end ?? now) : void 0;
    const chartPoints = lastPoint && currentAt !== void 0 && currentAt > lastPoint.at ? compactCounterSessionPoints([...sessionPoints2, { ...lastPoint, at: currentAt }]) : sessionPoints2;
    const startAt = sharedWindow?.start ?? state.session?.startedAt ?? chartPoints[0]?.at;
    const endAt = sharedWindow?.end ?? currentAt ?? startAt;
    renderSignalChart(svg, {
      points: chartPoints,
      geometry: "compact",
      rawStep: true,
      ...startAt === void 0 ? {} : { startAt },
      ...endAt === void 0 ? {} : { endAt },
      showLatestMarker: chartPoints.length > 0,
      ...endAt === void 0 ? {} : { latestAt: endAt },
      emptyTitle: "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5909\u5316\u306F\u3042\u308A\u307E\u305B\u3093",
      emptyHint: "\u4FDD\u5B58\u6E08\u307F\u306E\u610F\u5473\u7D50\u679C\u304C\u5909\u5316\u3059\u308B\u3068\u3001\u8868\u793A\u958B\u59CB\u5F8C\u306E\u5168\u671F\u9593\u3092\u8868\u793A\u3057\u307E\u3059",
      title: "\u6A2A\u8EF8\u306F\u8868\u793A\u958B\u59CB\u5F8C\u306E\u6642\u9593\u3001\u7E26\u8EF8\u306F\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5024\u3067\u3059\u3002\u8868\u793A\u958B\u59CB\u5F8C\u306E\u5168\u671F\u9593\u3092\u6700\u59271,000\u70B9\u3067\u8868\u793A\u3057\u307E\u3059\u3002"
    });
    return sessionPoints2.length;
  }
  function counterSummaryText(state, plottedCount) {
    if (state.history?.status === "pending") {
      return "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5C65\u6B74\u3092\u8AAD\u307F\u8FBC\u3093\u3067\u3044\u307E\u3059\u3002";
    }
    if (state.history?.status === "unavailable") {
      return "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5C65\u6B74\u3092\u53D6\u5F97\u3067\u304D\u307E\u305B\u3093\u3002";
    }
    const history = availableCounterHistory(state);
    const latestValue = latestHistoryValue(history);
    return latestValue === void 0 ? "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5909\u5316\u306F\u3042\u308A\u307E\u305B\u3093\u3002" : `${formatNumber(latestValue)}\uFF08\u4FDD\u5B58\u6E08\u307F\u3001\u8868\u793A\u958B\u59CB\u5F8C\u306E${plottedCount}\u70B9\uFF0F\u6700\u59271,000\u70B9\uFF09`;
  }
  function counterPreviewMessage(state) {
    if (!state.persisted) return "\u4FDD\u5B58\u5F8C\u306B\u7D2F\u7A4D\u958B\u59CB\u3002";
    if (state.history?.status === "pending") {
      return "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5024\u3092\u8AAD\u307F\u8FBC\u3093\u3067\u3044\u307E\u3059\u3002";
    }
    if (state.history?.status === "unavailable") {
      return "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5C65\u6B74\u3092\u53D6\u5F97\u3067\u304D\u307E\u305B\u3093\u3002";
    }
    return latestHistoryValue(availableCounterHistory(state)) === void 0 ? "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5909\u5316\u306F\u3042\u308A\u307E\u305B\u3093\u3002" : "\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5024\u306F\u8868\u793A\u958B\u59CB\u5F8C\u306E\u5909\u5316\u30B0\u30E9\u30D5\u3067\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002";
  }
  function latestRuleOutcome(payload, unit, counterState = { persisted: false }) {
    const latest = latestPreviewPoint(payload);
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
      case "cumulative_counter": {
        const delta = counterWindowDelta(payload);
        const history = availableCounterHistory(counterState);
        const persistedTotal = latestHistoryValue(history);
        if (counterState.persisted) {
          if (counterState.history?.status === "pending") {
            return {
              value: "\u7D2F\u7A4D\u5024\u3092\u8AAD\u307F\u8FBC\u307F\u4E2D",
              detail: `\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5024\u3092\u78BA\u8A8D\u3057\u3066\u3044\u307E\u3059\u3002\u3053\u306E\u8A2D\u5B9A\u306A\u3089\u76F4\u8FD160\u79D2\u3067 +${formatNumber(delta)}`,
              alarm: false
            };
          }
          if (counterState.history?.status === "unavailable") {
            return {
              value: "\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5024\u3092\u53D6\u5F97\u3067\u304D\u307E\u305B\u3093",
              detail: `\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5C65\u6B74\u3092\u53D6\u5F97\u3067\u304D\u307E\u305B\u3093\u3002\u3053\u306E\u8A2D\u5B9A\u306A\u3089\u76F4\u8FD160\u79D2\u3067 +${formatNumber(delta)}`,
              alarm: false
            };
          }
          if (persistedTotal === void 0) {
            return {
              value: "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5909\u5316\u306F\u3042\u308A\u307E\u305B\u3093",
              detail: `\u4FDD\u5B58\u6E08\u307F\u306E\u610F\u5473\u7D50\u679C\u304C\u5C4A\u304F\u3068\u7D2F\u7A4D\u5024\u3092\u8868\u793A\u3057\u307E\u3059\u3002\u3053\u306E\u8A2D\u5B9A\u306A\u3089\u76F4\u8FD160\u79D2\u3067 +${formatNumber(delta)}`,
              alarm: false
            };
          }
          return {
            value: `\u7D2F\u7A4D ${formatNumber(persistedTotal)}`,
            detail: `\u3053\u306E\u8A2D\u5B9A\u306A\u3089\u76F4\u8FD160\u79D2\u3067 +${formatNumber(delta)}`,
            alarm: false
          };
        }
        return {
          value: `\u76F4\u8FD160\u79D2\u3067 +${formatNumber(delta)}`,
          detail: "\u4FDD\u5B58\u5F8C\u306B\u7D2F\u7A4D\u958B\u59CB\u3002\u4FDD\u5B58\u6E08\u307F\u7D2F\u7A4D\u5024\u306F\u3053\u3053\u306B\u8868\u793A\u3055\u308C\u307E\u3059\u3002",
          alarm: false
        };
      }
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
  function renderRuleResult(panel, selected, state, unit, counterState) {
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
          "\u4FDD\u5B58\u6E08\u307F\u30EB\u30FC\u30EB\u3092\u9078\u629E\u3059\u308B\u3068\u5224\u5B9A\u7D50\u679C\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
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
        ],
        pending: [
          "\u8A2D\u5B9A\u7D50\u679C\u3092\u518D\u8A08\u7B97\u3057\u3066\u3044\u307E\u3059",
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
    const outcome = latestRuleOutcome(selected, unit, counterState);
    setText(name, selected.display_name);
    setText(kind, kindLabel(selected.kind));
    setText(value, outcome.value);
    setText(detail, outcome.detail);
    container.classList.toggle("is-alarm", outcome.alarm);
    return outcome;
  }
  function clearAuxiliaryOutputs(summary, state) {
    const messages = {
      none: "\u30B0\u30E9\u30D5\u306B\u8868\u793A\u3067\u304D\u308B\u53D7\u4FE1\u30C7\u30FC\u30BF\u306F\u307E\u3060\u3042\u308A\u307E\u305B\u3093\u3002",
      invalid: "\u8A2D\u5B9A\u5185\u5BB9\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002",
      error: "\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
    };
    if (summary) setText(summary, messages[state]);
  }
  function updateAccessibleSummary(summary, raw, selected, outcome, unit, plotPoints = raw.points ?? [], sessionWide = false) {
    if (!summary) return;
    const points = plotPoints;
    const period = sessionWide ? "\u753B\u9762\u3092\u958B\u3044\u3066\u304B\u3089\u73FE\u5728\u307E\u3067\uFF08\u5168\u671F\u9593\uFF09" : "\u76F4\u8FD160\u79D2";
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
      const evaluatedCount2 = raw.input_count ?? points.length;
      const bucketCount2 = points.length;
      setText(
        summary,
        `\u53D7\u4FE1\u5024\u306F${formatNumber(Math.min(...inputs2))}\u304B\u3089${formatNumber(Math.max(...inputs2))}\u3067\u3059\u3002\u9078\u629E\u4E2D\u306F${selected.display_name}\u3001${kindLabel(selected.kind)}\u3067\u3059\u304C\u3001\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002${evaluatedCount2}\u4EF6\u3092\u8A55\u4FA1\u3057\u3001${period}\u306E${bucketCount2}bucket\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002`
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
    const evaluatedCount = raw.input_count ?? points.length;
    const bucketCount = points.length;
    const ruleText = selected && outcome ? selected.kind === "cumulative_counter" ? `\u9078\u629E\u4E2D\u306F${selected.display_name}\u3001${kindLabel(selected.kind)}\u3001${outcome.value}\u3067\u3059\u3002${outcome.detail}` : `\u9078\u629E\u4E2D\u306F${selected.display_name}\u3001${kindLabel(selected.kind)}\u3001\u73FE\u5728\u306F${outcome.value}\u3067\u3059\u3002` : "\u9078\u629E\u4E2D\u306E\u30EB\u30FC\u30EB\u306F\u3042\u308A\u307E\u305B\u3093\u3002";
    const calibratedText = selected?.kind === "numeric" && outcome ? (() => {
      const calibrated = points.flatMap((point) => [
        Number(point.calibrated_min),
        Number(point.calibrated_max)
      ]);
      const latest = latestPreviewPoint(selected);
      if (!latest || !calibrated.length) return "";
      return `\u88DC\u6B63\u5F8C\u306F${formatNumber(Math.min(...calibrated))}\u304B\u3089${formatNumber(Math.max(...calibrated))}\u3001\u6700\u65B0\u306E\u88DC\u6B63\u5F8C\u306F${formatNumber(latest.calibrated)}${unit ? ` ${unit}` : ""}\u3067\u3059\u3002`;
    })() : "";
    setText(
      summary,
      `\u53D7\u4FE1\u5024\u306F${formatNumber(Math.min(...inputs))}\u304B\u3089${formatNumber(Math.max(...inputs))}\u3067\u3059\u3002${calibratedText}${ruleText}${evaluatedCount}\u4EF6\u3092\u8A55\u4FA1\u3057\u3001${period}\u306E${bucketCount}bucket\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002`
    );
  }
  function previewWindow(payload, points) {
    const now = Date.now();
    const end = payload.window_end ?? (points.at(-1) ? pointPlotAt(points.at(-1)) : now);
    return {
      start: payload.window_start ?? (points[0] ? pointPlotAt(points[0]) : end),
      end
    };
  }
  function renderPreviewChart(svg, payload, showSemanticOverlays, unit, rawBoolean, showResult, sharedWindow, sessionWide = false) {
    const points = payload.points ?? [];
    const window2 = sharedWindow ?? previewWindow(payload, points);
    renderSignalChart(svg, {
      points: points.map((point) => ({
        at: pointPlotAt(point),
        value: point.input,
        minimum: point.input_min,
        maximum: point.input_max,
        sampleCount: point.sample_count,
        result: point.calibrated,
        resultMinimum: point.calibrated_min,
        resultMaximum: point.calibrated_max,
        activeRatio: point.sample_count ? Number(point.active_samples ?? 0) / point.sample_count : 0
      })),
      geometry: "compact",
      unit,
      boolean: rawBoolean && !showSemanticOverlays,
      rawStep: rawBoolean,
      startAt: window2.start,
      endAt: window2.end,
      showResult,
      resultStep: showResult && rawBoolean,
      showLatestMarker: showSemanticOverlays,
      showActiveBands: showSemanticOverlays && payload.kind !== "numeric" && payload.kind !== "cumulative_counter",
      thresholds: showSemanticOverlays ? { rise: payload.rise_threshold, fall: payload.fall_threshold } : void 0,
      emptyTitle: payload.error ? "\u3053\u306E\u30EB\u30FC\u30EB\u3067\u306F\u53D7\u4FE1\u5024\u3092\u5224\u5B9A\u3067\u304D\u307E\u305B\u3093" : "\u307E\u3060\u53D7\u4FE1\u30C7\u30FC\u30BF\u304C\u3042\u308A\u307E\u305B\u3093",
      emptyHint: payload.error ? "\u5165\u529B\u5024\u306E\u88DC\u6B63\u3068\u5224\u5B9A\u6761\u4EF6\u3092\u78BA\u8A8D\u3057\u3066\u304F\u3060\u3055\u3044" : "\u5B9F\u969B\u306B\u5C4A\u3044\u305F\u5024\u3092\u5F85\u3063\u3066\u3044\u307E\u3059",
      title: `\u6A2A\u8EF8\u306F${sessionWide ? "\u753B\u9762\u3092\u958B\u3044\u3066\u304B\u3089\u73FE\u5728\u307E\u3067" : "\u76F4\u8FD160\u79D2"}\u3001\u7E26\u8EF8\u306F\u53D7\u4FE1\u5024${unit ? `\uFF08${unit}\uFF09` : ""}${showResult ? "\u3068\u8A2D\u5B9A\u7D50\u679C" : ""}\u3067\u3059\u3002`
    });
    return points;
  }
  function isMultipleRulePreview(response) {
    return "rules" in response;
  }
  function activeSettingPanel(scope) {
    return queryAll(
      "[data-setting-panel]",
      scope
    ).find((panel) => !panel.hidden);
  }
  function previewTargetID(target) {
    return target.dataset.ruleId || query("form.semantic-form[data-preview-id]", target)?.dataset.previewId;
  }
  function previewTargets(panel) {
    return queryAll(
      "details[data-preview-target]",
      panel
    ).filter((target) => !!previewTargetID(target));
  }
  function persistedPreviewTargets(panel) {
    return previewTargets(panel).filter((target) => !!target.dataset.ruleId);
  }
  function previewTargetLabel(target) {
    const form = query("form.semantic-form", target);
    const name = form ? formField(form, "display_name")?.value.trim() : void 0;
    const targetID = previewTargetID(target);
    return name || (targetID === "draft-alarm" ? "\u65B0\u3057\u3044\u7570\u5E38\u691C\u77E5" : targetID === "draft-normal" ? "\u65B0\u3057\u3044\u8A08\u6E2C\u30EB\u30FC\u30EB" : void 0) || query("summary strong", target)?.textContent?.trim() || targetID || "\u30EB\u30FC\u30EB";
  }
  function renderPreviewRuleOptions(selector, targets, selectedID) {
    if (!selector) return;
    selector.replaceChildren();
    if (!targets.length) {
      const option = document.createElement("option");
      option.value = "";
      option.textContent = "\u9078\u629E\u3067\u304D\u308B\u30EB\u30FC\u30EB\u306A\u3057";
      option.disabled = true;
      option.selected = true;
      selector.append(option);
      selector.disabled = true;
      return;
    }
    const hasPersistedTarget = targets.some((target) => !!target.dataset.ruleId);
    if (!hasPersistedTarget) {
      const option = document.createElement("option");
      option.value = "";
      option.textContent = "\u30EB\u30FC\u30EB\u3092\u9078\u629E";
      option.disabled = true;
      option.selected = !selectedID;
      selector.append(option);
    }
    selector.disabled = false;
    for (const target of targets) {
      const option = document.createElement("option");
      option.value = previewTargetID(target) ?? "";
      option.textContent = previewTargetLabel(target);
      selector.append(option);
    }
    const selectedTarget = targets.find(
      (target) => previewTargetID(target) === selectedID
    );
    const firstPersistedTarget = targets.find((target) => !!target.dataset.ruleId);
    selector.value = selectedTarget ? previewTargetID(selectedTarget) ?? "" : firstPersistedTarget ? previewTargetID(firstPersistedTarget) ?? "" : "";
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
      rise_threshold: void 0,
      fall_threshold: void 0,
      points: (payload.points ?? []).map(rawPreviewPoint),
      latest_point: payload.latest_point ? rawPreviewPoint(payload.latest_point) : void 0
    };
  }
  function buildRequest(signalRef, forms, calibrationForm, multipleRules, activeID) {
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
    const rawBoolean = forms.some(
      (form) => form.dataset.booleanInput === "true"
    );
    const range = query("[data-preview-range]", panel);
    const count = query("[data-preview-count]", panel);
    const message = query("[data-preview-message]", panel);
    const feedState = query("[data-preview-feed-state]", panel);
    const checkedAt = query("[data-preview-checked-at]", panel);
    const toggle = query("[data-preview-toggle]", panel);
    const chart = query("[data-preview-chart]", panel);
    const accessibleSummaryID = chart?.getAttribute("aria-describedby");
    const accessibleSummary = accessibleSummaryID ? document.getElementById(accessibleSummaryID) : null;
    const counterPanel = query("[data-preview-counter]", panel);
    const counterChart = query(
      "[data-preview-counter-chart]",
      panel
    );
    const counterSummary = query(
      "[data-preview-counter-summary]",
      panel
    );
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
    const ruleSelector = query(
      "[data-preview-rule-select]",
      panel
    );
    const unit = panel.dataset.unit ?? "";
    if (!range || !count || !message || !chart) return;
    const clockStartedAt = Date.now();
    const monotonicStartedAt = performance.now();
    const edgeNow = () => Math.floor(
      clockStartedAt + Math.max(0, performance.now() - monotonicStartedAt)
    );
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
    let controllerResultKey;
    let counterHistoryController;
    let counterHistoryRuleID;
    let counterHistory;
    let counterHistorySession;
    let lastAvailableCounterHistory;
    let renderCurrentCounter;
    let debounce;
    let previewUnavailable = false;
    let paused = false;
    let lastSeenReceivedAt;
    let selectedPreviewID;
    let previewResultKey;
    let previewGeneration = 0;
    let lastRawPreview;
    const previewSession = {
      startedAt: edgeNow(),
      points: { archive: [], recent: [] }
    };
    const lastSelectedTargetByPanel = /* @__PURE__ */ new WeakMap();
    const pendingInitialToggleStates = /* @__PURE__ */ new Map();
    const setCounterPanel = (visible) => {
      if (counterPanel) counterPanel.hidden = !visible;
    };
    const setSemanticLegends = (semanticVisible, resultVisible, payload) => {
      if (resultLegend) resultLegend.hidden = !resultVisible;
      if (thresholdLegend) {
        thresholdLegend.hidden = !semanticVisible || !payload || !isFiniteNumber(payload.rise_threshold) && !isFiniteNumber(payload.fall_threshold);
      }
    };
    const hideSemanticAuxiliaries = () => {
      setSemanticLegends(false, false);
      setCounterPanel(false);
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
    const selectPreviewTarget = (selectedTarget) => {
      const activePanel = activeSettingPanel(previewScope);
      const targets = activePanel ? previewTargets(activePanel) : [];
      const selectedID = selectedTarget ? previewTargetID(selectedTarget) : void 0;
      if (!activePanel || !selectedTarget || !selectedID || !targets.includes(selectedTarget)) {
        selectedPreviewID = void 0;
        if (activePanel) lastSelectedTargetByPanel.delete(activePanel);
        renderPreviewRuleOptions(ruleSelector, targets, void 0);
        return;
      }
      selectedPreviewID = selectedID;
      lastSelectedTargetByPanel.set(activePanel, selectedID);
      renderPreviewRuleOptions(ruleSelector, targets, selectedID);
      for (const target of queryAll(
        "details[data-preview-target]",
        activePanel
      )) {
        target.open = target === selectedTarget;
      }
    };
    const restorePreviewTarget = () => {
      const activePanel = activeSettingPanel(previewScope);
      const targets = activePanel ? previewTargets(activePanel) : [];
      const persistedTargets = activePanel ? persistedPreviewTargets(activePanel) : [];
      const rememberedID = activePanel ? lastSelectedTargetByPanel.get(activePanel) : void 0;
      selectPreviewTarget(
        targets.find((target) => previewTargetID(target) === rememberedID) ?? persistedTargets[0]
      );
    };
    const refreshRuleSelectorLabels = (form) => {
      const activePanel = activeSettingPanel(previewScope);
      if (!activePanel || !activePanel.contains(form)) return;
      const targets = previewTargets(activePanel);
      const selectedID = targets.some(
        (target) => previewTargetID(target) === selectedPreviewID
      ) ? selectedPreviewID : void 0;
      renderPreviewRuleOptions(ruleSelector, targets, selectedID);
    };
    const counterStateFor = (ruleID) => {
      if (!ruleID) {
        counterHistoryController?.abort();
        counterHistoryController = void 0;
        counterHistoryRuleID = void 0;
        counterHistory = void 0;
        counterHistorySession = void 0;
        lastAvailableCounterHistory = void 0;
        renderCurrentCounter = void 0;
        return { persisted: false };
      }
      if (counterHistoryRuleID !== ruleID) {
        counterHistoryController?.abort();
        counterHistoryController = void 0;
        renderCurrentCounter = void 0;
        counterHistoryRuleID = ruleID;
        counterHistory = { status: "pending" };
        counterHistorySession = {
          startedAt: edgeNow(),
          baselineCaptured: false,
          points: []
        };
        lastAvailableCounterHistory = void 0;
      }
      return {
        persisted: true,
        history: counterHistory ?? { status: "pending" },
        session: counterHistorySession
      };
    };
    const previewResultIdentity = (activeID = selectedPreviewID) => {
      const activeForm = forms.find(
        (candidate) => candidate.dataset.previewId === activeID
      );
      if (!activeID || !activeForm) return void 0;
      return JSON.stringify({
        activeID,
        calibration: {
          scale: calibrationForm ? numericFormField(calibrationForm, "scale") : 1,
          offset: calibrationForm ? numericFormField(calibrationForm, "offset") : 0
        },
        spec: ruleSpec(activeForm)
      });
    };
    const renderCachedRaw = (state) => {
      if (!lastRawPreview) return;
      const activeCounterRuleID = counterRuleIDForActiveForm(
        forms,
        selectedPreviewID
      );
      const persisted = Boolean(
        activeCounterRuleID && activeCounterRuleID === counterHistoryRuleID
      );
      const points = persisted ? previewSessionPoints(previewSession, void 0) : lastRawPreview.points ?? [];
      const now = edgeNow();
      const window2 = persisted ? { start: previewSession.startedAt, end: now } : previewWindow(lastRawPreview, points);
      renderPreviewChart(
        chart,
        {
          ...lastRawPreview,
          points,
          window_start: window2.start,
          window_end: window2.end
        },
        false,
        unit,
        rawBoolean,
        false,
        window2,
        persisted
      );
      if (persisted && state && counterChart) {
        const plottedCount = renderCounterHistoryChart(
          counterChart,
          state,
          now,
          window2
        );
        setCounterPanel(true);
        if (counterSummary) {
          setText(counterSummary, counterSummaryText(state, plottedCount));
        }
      }
    };
    const synchronizePreviewResult = () => {
      const key = previewResultIdentity();
      if (key === previewResultKey) return key;
      previewResultKey = key;
      previewGeneration += 1;
      invalidatePreviewResults(previewSession);
      controller?.abort();
      setSemanticLegends(false, false);
      const activeCounterRuleID = counterRuleIDForActiveForm(
        forms,
        selectedPreviewID
      );
      if (activeCounterRuleID !== counterHistoryRuleID) {
        counterStateFor(activeCounterRuleID);
        setCounterPanel(false);
      }
      const state = activeCounterRuleID ? counterStateFor(activeCounterRuleID) : void 0;
      if (state?.persisted && renderCurrentCounter) {
        renderCurrentCounter(state);
      } else {
        renderCachedRaw(state);
      }
      if (lastRawPreview) renderRuleResult(panel, null, "pending", unit);
      return key;
    };
    const refreshCounterHistory = (ruleID) => {
      if (counterHistoryController || counterHistoryRuleID !== ruleID) return;
      const historyController = new AbortController();
      counterHistoryController = historyController;
      const requestAt = edgeNow();
      void loadCounterHistory(ruleID, historyController.signal, requestAt).then((history) => {
        if (counterHistoryController !== historyController || counterHistoryRuleID !== ruleID || historyController.signal.aborted) {
          return;
        }
        counterHistoryController = void 0;
        if (history.status === "available") {
          const value = retainLatestHistory(lastAvailableCounterHistory, history.value);
          lastAvailableCounterHistory = value;
          counterHistory = { status: "available", value };
          if (counterHistorySession) {
            counterHistorySession = mergeCounterHistorySession(
              counterHistorySession,
              history.value,
              edgeNow()
            );
          }
          renderCurrentCounter?.({
            persisted: true,
            history: counterHistory,
            session: counterHistorySession
          });
          return;
        }
        counterHistory = history;
        renderCurrentCounter?.({
          persisted: true,
          history,
          session: counterHistorySession
        });
      }).catch(() => {
        if (counterHistoryController !== historyController || counterHistoryRuleID !== ruleID || historyController.signal.aborted) {
          return;
        }
        counterHistoryController = void 0;
        const unavailable = { status: "unavailable" };
        counterHistory = unavailable;
        renderCurrentCounter?.({
          persisted: true,
          history: unavailable,
          session: counterHistorySession
        });
      });
    };
    const refresh = async (force = false) => {
      const activeID = selectedPreviewID;
      const resultKey = synchronizePreviewResult();
      if (!force && controller && controllerResultKey === resultKey) return;
      const generation = previewGeneration;
      controller?.abort();
      const requestController = new AbortController();
      controller = requestController;
      controllerResultKey = resultKey;
      clearFieldErrors(previewScope);
      const requestedCounterRuleID = counterRuleIDForActiveForm(forms, activeID);
      counterStateFor(requestedCounterRuleID);
      if (requestedCounterRuleID) {
        refreshCounterHistory(requestedCounterRuleID);
      }
      const body = buildRequest(
        signalRef,
        forms,
        calibrationForm,
        multipleRules,
        activeID
      );
      try {
        const result = await createMappingPreview(
          body,
          csrfToken(),
          requestController.signal
        );
        if (controller !== requestController || requestController.signal.aborted || generation !== previewGeneration || resultKey !== previewResultKey) {
          return;
        }
        if (!result.ok) {
          renderCurrentCounter = void 0;
          hideSemanticAuxiliaries();
          const fieldName = result.error?.error.field;
          const activeForm = forms.find(
            (candidate) => candidate.dataset.previewId === activeID
          );
          const invalidField = fieldName && activeForm ? formField(activeForm, fieldName) : null;
          const fieldLabel = invalidField?.closest("label")?.querySelector(":scope > span")?.textContent?.trim();
          if (result.status === 404 && !forms[0]) {
            previewUnavailable = true;
            renderRuleResult(panel, null, "none", unit);
            clearAuxiliaryOutputs(accessibleSummary, "none");
            setFeedState("\u8868\u793A\u3059\u308B\u30EB\u30FC\u30EB\u304C\u3042\u308A\u307E\u305B\u3093");
            setText(
              message,
              "\u5024\u306E\u5909\u63DB\u304C\u8A2D\u5B9A\u3055\u308C\u308B\u3068\u3001\u3053\u3053\u306B\u8A2D\u5B9A\u7D50\u679C\u3092\u8868\u793A\u3057\u307E\u3059\u3002"
            );
          } else if (fieldLabel && invalidField) {
            renderRuleResult(panel, null, "invalid", unit);
            clearAuxiliaryOutputs(accessibleSummary, "invalid");
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
          renderCurrentCounter = void 0;
          hideSemanticAuxiliaries();
          renderRuleResult(
            panel,
            null,
            activeID ? "error" : "none",
            unit
          );
          clearAuxiliaryOutputs(
            accessibleSummary,
            activeID ? "error" : "none"
          );
          setFeedState("\u8868\u793A\u3059\u308B\u30EB\u30FC\u30EB\u304C\u3042\u308A\u307E\u305B\u3093");
          setText(message, "\u78BA\u8A8D\u3067\u304D\u308B\u30EB\u30FC\u30EB\u304C\u3042\u308A\u307E\u305B\u3093\u3002");
          return;
        }
        const rawPayload = rawOnlyPreview(selection.raw ?? payload);
        lastRawPreview = rawPayload;
        const rawPoints = rawPayload.points ?? [];
        const rawWindow = previewWindow(rawPayload, rawPoints);
        mergePreviewSession(
          previewSession,
          rawPoints,
          selectedReady?.points ?? void 0,
          selectedReady ? resultKey : void 0
        );
        const persistedRuleID = persistedCounterRuleID(
          forms,
          activeID,
          selectedReady
        );
        const counterState = counterStateFor(persistedRuleID);
        if (!persistedRuleID) setCounterPanel(false);
        const renderPreviewMessage = (state, points, window2, showResult) => {
          if (payload.input_count === 0) {
            range.textContent = "\u53D7\u4FE1\u30C7\u30FC\u30BF\u306F\u307E\u3060\u3042\u308A\u307E\u305B\u3093";
            count.textContent = "\u53D7\u4FE1\u5024\u304C\u5C4A\u304F\u3068\u8A2D\u5B9A\u7D50\u679C\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002";
            setText(message, "\u5C65\u6B74\u306F\u4F5C\u3089\u305A\u3001\u5B9F\u969B\u306B\u5C4A\u3044\u305F\u5024\u3060\u3051\u3092\u8868\u793A\u3057\u307E\u3059\u3002");
          } else {
            range.textContent = `${persistedRuleID ? "\u8868\u793A\u958B\u59CB\u5F8C" : "\u76F4\u8FD1"}${formatDuration(window2.start, window2.end)}\u306E\u53D7\u4FE1\u5024`;
            count.textContent = `${payload.input_count.toLocaleString("ja-JP")}\u4EF6\u3092\u8A55\u4FA1\u3057\u3001${persistedRuleID ? "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u5168\u671F\u9593" : "\u76F4\u8FD160\u79D2"}\u306E${points.length.toLocaleString("ja-JP")}bucket\u3092\u8868\u793A`;
            setText(
              message,
              payload.truncated_by === "input_count" ? "\u9AD8\u901F\u306A\u4FE1\u53F7\u306E\u305F\u3081\u3001\u6700\u65B020,000\u4EF6\u3092\u8981\u7D04\u3057\u3066\u3044\u307E\u3059\u3002" : payload.kind === "cumulative_counter" ? `\u3053\u306E\u8A2D\u5B9A\u306A\u3089\u76F4\u8FD160\u79D2\u3067 +${formatNumber(counterWindowDelta(payload))}\u3002` + counterPreviewMessage(state) : !showResult ? "\u5909\u63DB\u524D\u5F8C\u306E\u5024\u306F\u540C\u3058\u3067\u3059\u3002\u88DC\u6B63\u3092\u5909\u66F4\u3059\u308B\u3068\u5DEE\u3092\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002" : "\u8A2D\u5B9A\u3092\u5909\u3048\u308B\u3068\u3001\u4FDD\u5B58\u524D\u306E\u7D50\u679C\u3092\u3053\u306E\u30B0\u30E9\u30D5\u3067\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
            );
          }
          if (selectedFailure) {
            setText(
              message,
              "\u5224\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u53D7\u4FE1\u5024\u306F\u305D\u306E\u307E\u307E\u78BA\u8A8D\u3067\u304D\u307E\u3059\u3002"
            );
          }
        };
        const renderCounterState = (state) => {
          if (generation !== previewGeneration || resultKey !== previewResultKey) {
            setSemanticLegends(false, false);
            renderCachedRaw(state);
            if (lastRawPreview) renderRuleResult(panel, null, "pending", unit);
            return;
          }
          const now = edgeNow();
          const points = persistedRuleID ? previewSessionPoints(
            previewSession,
            selectedReady ? resultKey : void 0
          ) : payload.points ?? [];
          const window2 = persistedRuleID ? {
            start: previewSession.startedAt,
            end: now
          } : previewWindow(payload, points);
          const showResult = Boolean(selectedReady) && hasMeaningfulResult(points);
          const chartPayload = {
            ...payload,
            points,
            window_start: window2.start,
            window_end: window2.end
          };
          setSemanticLegends(Boolean(selectedReady), showResult, payload);
          const plottedPoints = renderPreviewChart(
            chart,
            chartPayload,
            Boolean(selectedReady),
            unit,
            rawBoolean,
            showResult,
            window2,
            Boolean(persistedRuleID)
          );
          if (persistedRuleID && counterChart) {
            const plottedCount = renderCounterHistoryChart(
              counterChart,
              state,
              now,
              window2
            );
            setCounterPanel(true);
            if (counterSummary) {
              setText(counterSummary, counterSummaryText(state, plottedCount));
            }
          }
          const resultState = !activeID ? "none" : selectedReady ? "ready" : "error";
          const outcome = renderRuleResult(
            panel,
            selectedReady ?? selectedFailure,
            resultState,
            unit,
            state
          );
          updateAccessibleSummary(
            accessibleSummary,
            chartPayload,
            selectedReady ?? selectedFailure,
            outcome,
            unit,
            plottedPoints,
            Boolean(persistedRuleID)
          );
          renderPreviewMessage(state, plottedPoints, window2, showResult);
        };
        renderCurrentCounter = persistedRuleID ? renderCounterState : void 0;
        renderCounterState(counterState);
        if (persistedRuleID) {
          refreshCounterHistory(persistedRuleID);
        }
        const latest = selection.raw ? latestPreviewPoint(selection.raw) : void 0;
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
          const elapsed = Math.max(0, edgeNow() - latest.received_at);
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
      } catch (error) {
        if (!(error instanceof DOMException && error.name === "AbortError")) {
          renderCurrentCounter = void 0;
          hideSemanticAuxiliaries();
          renderRuleResult(panel, null, "error", unit);
          clearAuxiliaryOutputs(accessibleSummary, "error");
          setFeedState("\u66F4\u65B0\u3092\u78BA\u8A8D\u3067\u304D\u307E\u305B\u3093");
          setText(
            message,
            "\u8A2D\u5B9A\u7D50\u679C\u3092\u66F4\u65B0\u3067\u304D\u307E\u305B\u3093\u3002\u30C7\u30FC\u30BF\u53D7\u4FE1\u306B\u306F\u5F71\u97FF\u3042\u308A\u307E\u305B\u3093\u3002"
          );
        }
      } finally {
        if (controller === requestController) {
          controller = void 0;
          controllerResultKey = void 0;
        }
      }
    };
    const schedule = () => {
      if (debounce !== void 0) window.clearTimeout(debounce);
      debounce = window.setTimeout(() => void refresh(true), 300);
    };
    restorePreviewTarget();
    for (const form of forms) {
      const onFormChange = (event) => {
        if (event.target instanceof HTMLInputElement && event.target.name === "display_name") {
          refreshRuleSelectorLabels(form);
        }
        synchronizePreviewResult();
        schedule();
      };
      form.addEventListener("input", onFormChange);
      form.addEventListener("change", onFormChange);
    }
    const scheduleAfterIdentityChange = () => {
      synchronizePreviewResult();
      schedule();
    };
    calibrationForm?.addEventListener("input", scheduleAfterIdentityChange);
    calibrationForm?.addEventListener("change", scheduleAfterIdentityChange);
    previewScope.addEventListener(SETTING_TAB_CHANGE_EVENT, () => {
      restorePreviewTarget();
      scheduleAfterIdentityChange();
    });
    ruleSelector?.addEventListener("change", () => {
      const activePanel = activeSettingPanel(previewScope);
      const selectedTarget = activePanel ? previewTargets(activePanel).find(
        (target) => previewTargetID(target) === ruleSelector.value
      ) : void 0;
      selectPreviewTarget(selectedTarget);
      scheduleAfterIdentityChange();
    });
    for (const target of queryAll(
      "details[data-preview-target]",
      previewScope
    )) {
      pendingInitialToggleStates.set(target, target.open);
      target.addEventListener("toggle", () => {
        const initialOpen = pendingInitialToggleStates.get(target);
        if (initialOpen !== void 0) {
          pendingInitialToggleStates.delete(target);
          if (target.open === initialOpen) return;
        }
        const activePanel = activeSettingPanel(previewScope);
        if (target.open && activePanel?.contains(target) && previewTargets(activePanel).includes(target)) {
          selectPreviewTarget(target);
        }
        scheduleAfterIdentityChange();
      });
    }
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
    synchronizePreviewResult();
    void refresh();
    window.setInterval(() => {
      if (document.visibilityState === "visible" && !previewUnavailable && !paused) {
        void refresh();
        if (counterHistoryRuleID) {
          renderCurrentCounter?.(counterStateFor(counterHistoryRuleID));
        }
      }
    }, 1e3);
  }
  function initializePreviews() {
    for (const panel of queryAll("[data-setting-simulation]")) {
      initializePreview(panel);
    }
  }

  // src/live.ts
  var REFRESH_MS = 5 * 1e3;
  var BUCKET_MS = 1e3;
  var MAX_HISTORY_BUCKETS = 1e3;
  var MAX_NUMERIC_POINTS = MAX_HISTORY_BUCKETS;
  var MAX_BOOLEAN_POINTS = MAX_HISTORY_BUCKETS;
  var MAX_ACTIVE_CARDS = 12;
  function formatNumber2(value, decimalPlaces = 1) {
    return value.toLocaleString("ja-JP", {
      maximumFractionDigits: Math.max(0, decimalPlaces)
    });
  }
  function isBooleanKind(kind) {
    return kind === "bool" || kind === "boolean" || kind === "alarm";
  }
  function isStepKind(kind) {
    return isBooleanKind(kind) || kind === "cumulative_counter";
  }
  function relativeTime(receivedAt, now) {
    const elapsed = Math.max(0, now - receivedAt);
    if (elapsed < 1e4) return "\u305F\u3063\u305F\u4ECA";
    if (elapsed < 6e4) return `${Math.floor(elapsed / 1e3)}\u79D2\u524D`;
    if (elapsed < 60 * 6e4) {
      const minutes = Math.floor(elapsed / 6e4);
      const seconds = Math.floor(elapsed % 6e4 / 1e3);
      return `${minutes}\u5206${seconds}\u79D2\u524D`;
    }
    return `${Math.floor(elapsed / (60 * 6e4))}\u6642\u9593\u524D`;
  }
  function historyWindow(from, to) {
    const duration = Math.max(BUCKET_MS, to - from);
    const bucketMs = Math.max(
      BUCKET_MS,
      Math.ceil(duration / MAX_HISTORY_BUCKETS / BUCKET_MS) * BUCKET_MS
    );
    return { from, to, bucketMs };
  }
  function sessionPoints(payload, boolean, sessionStartedAt) {
    const points = payload.points.filter(
      (point) => point.bucket_start >= sessionStartedAt
    );
    if (!boolean) return points.slice(-MAX_NUMERIC_POINTS);
    const transitions = [];
    for (const point of points) {
      const state = (point.last_value ?? point.average) >= 0.5 ? 1 : 0;
      if (transitions.at(-1)?.average === state) continue;
      transitions.push({
        ...point,
        minimum: state,
        average: state,
        maximum: state
      });
    }
    return transitions.slice(-MAX_BOOLEAN_POINTS);
  }
  function renderChart(svg, payload, kind, unit, now, sessionStartedAt) {
    const boolean = isBooleanKind(kind);
    const step = isStepKind(kind);
    const points = sessionPoints(payload, boolean, sessionStartedAt);
    const chartPoints = points.map((point) => ({
      at: point.bucket_start,
      value: kind === "cumulative_counter" ? point.last_value ?? point.average : point.average,
      minimum: point.minimum,
      maximum: point.maximum,
      sampleCount: point.sample_count
    }));
    return renderSignalChart(svg, {
      points: chartPoints,
      geometry: "compact",
      unit,
      boolean,
      rawStep: step,
      startAt: sessionStartedAt,
      endAt: Math.max(now, sessionStartedAt + 1e3),
      latestAt: payload.latest_received_at !== null && payload.latest_received_at >= sessionStartedAt ? payload.latest_received_at : points.at(-1)?.bucket_start,
      showLatestMarker: true,
      emptyTitle: "\u3053\u306E\u753B\u9762\u3092\u958B\u3044\u3066\u304B\u3089\u306E\u53D7\u4FE1\u3092\u5F85\u3063\u3066\u3044\u307E\u3059",
      emptyHint: "\u8868\u793A\u958B\u59CB\u5F8C\u306E\u5168\u671F\u9593\u3092\u6700\u59271,000bucket\u3067\u8868\u793A\u3057\u307E\u3059",
      title: boolean ? "\u6A2A\u8EF8\u306F\u3053\u306E\u753B\u9762\u3092\u958B\u3044\u3066\u304B\u3089\u306E\u5168\u671F\u9593\uFF08\u6700\u59271,000bucket\uFF09\u3001\u7E26\u8EF8\u306F\u63A5\u70B9\u306EON/OFF\u3067\u3059\u3002" : `\u6A2A\u8EF8\u306F\u3053\u306E\u753B\u9762\u3092\u958B\u3044\u3066\u304B\u3089\u306E\u5168\u671F\u9593\uFF08\u6700\u59271,000bucket\uFF09\u3001\u7E26\u8EF8\u306F\u5024${unit ? `\uFF08${unit}\uFF09` : ""}\u3067\u3059\u3002`
    });
  }
  function setStatus(card, label, className) {
    const status = query("[data-live-status]", card);
    if (!status) return;
    status.textContent = label;
    status.className = `status-pill ${className}`;
  }
  function retainCardLatest(previous, payload) {
    if (payload.latest_received_at !== null || previous?.latest_received_at === null || !previous) {
      return payload;
    }
    return {
      ...payload,
      latest_received_at: previous.latest_received_at,
      latest_value: previous.latest_value
    };
  }
  function renderCard(card, payload, now, staleAfterMs, sessionStartedAt) {
    const kind = card.dataset.valueKind ?? payload.value_type;
    const boolean = isBooleanKind(kind);
    const unit = card.dataset.unit ?? payload.unit;
    const decimalPlaces = Number(card.dataset.decimalPlaces ?? 1);
    const value = query("[data-live-value]", card);
    const received = query("[data-live-received]", card);
    const summary = query("[data-live-summary]", card);
    const chart = query("[data-live-chart]", card);
    const pointCount = chart ? renderChart(chart, payload, kind, unit, now, sessionStartedAt) : 0;
    if (payload.latest_received_at === null) {
      setStatus(card, "\u672A\u53D7\u4FE1", "never");
      if (value) value.textContent = "\u2014";
      if (received) received.textContent = "\u307E\u3060\u53D7\u4FE1\u3057\u3066\u3044\u307E\u305B\u3093";
    } else {
      const relative = relativeTime(payload.latest_received_at, now);
      const stale = now - payload.latest_received_at > staleAfterMs;
      setStatus(card, stale ? "\u8981\u78BA\u8A8D" : "\u53D7\u4FE1\u4E2D", stale ? "stale" : "receiving");
      if (received) {
        received.textContent = `\u6700\u7D42\u53D7\u4FE1 ${relative}`;
        received.title = new Date(payload.latest_received_at).toLocaleString("ja-JP");
      }
      if (value) {
        if (boolean && (typeof payload.latest_value === "boolean" || typeof payload.latest_value === "number")) {
          value.textContent = (typeof payload.latest_value === "boolean" ? payload.latest_value : payload.latest_value >= 0.5) ? "ON" : "OFF";
        } else if (typeof payload.latest_value === "number") {
          value.textContent = `${formatNumber2(payload.latest_value, decimalPlaces)}${unit ? ` ${unit}` : ""}`;
        }
      }
    }
    if (summary) {
      summary.textContent = boolean ? `\u3053\u306E\u753B\u9762\u3092\u958B\u3044\u3066\u304B\u3089${pointCount}\u4EF6\u306E\u72B6\u614B\u5909\u5316\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002\u5168\u671F\u9593\u30FB\u6700\u59271,000bucket\u3001\u7E26\u8EF8\u306FON/OFF\u3067\u3059\u3002` : `\u3053\u306E\u753B\u9762\u3092\u958B\u3044\u3066\u304B\u3089${pointCount}\u4EF6\u3092\u8868\u793A\u3057\u3066\u3044\u307E\u3059\u3002\u5168\u671F\u9593\u30FB\u6700\u59271,000bucket\u3001\u7E26\u8EF8\u306F\u5024${unit ? `\uFF08${unit}\uFF09` : ""}\u3067\u3059\u3002`;
    }
  }
  function activeCards(dashboard) {
    const viewportHeight = window.innerHeight || document.documentElement.clientHeight;
    return queryAll("[data-live-signal]", dashboard).filter((card) => {
      const bounds = card.getBoundingClientRect();
      return bounds.bottom >= 0 && bounds.top <= viewportHeight;
    }).slice(0, MAX_ACTIVE_CARDS);
  }
  function initializeLiveDashboard() {
    const dashboard = query("[data-live-dashboard]");
    const state = query("[data-live-dashboard-state]");
    if (!dashboard) return;
    const staleAfterMs = Number(dashboard.dataset.staleAfterMs ?? 3e5);
    const sessionStartedAt = Number(dashboard.dataset.liveSessionStartedAt);
    if (!Number.isFinite(sessionStartedAt)) {
      if (state) state.textContent = "\u30E9\u30A4\u30D6\u66F4\u65B0\u3092\u958B\u59CB\u3067\u304D\u307E\u305B\u3093";
      return;
    }
    const pageOpenedAt = performance.now();
    const edgeNow = () => Math.floor(sessionStartedAt + Math.max(0, performance.now() - pageOpenedAt));
    const snapshotAt = Number(dashboard.dataset.liveSnapshotAt);
    const liveSnapshotAt = Number.isFinite(snapshotAt) && snapshotAt >= 0 && snapshotAt <= sessionStartedAt ? snapshotAt : sessionStartedAt;
    const latestPayloads = /* @__PURE__ */ new WeakMap();
    const catchUpComplete = /* @__PURE__ */ new WeakSet();
    let controller = null;
    const refresh = async () => {
      if (!dashboard.isConnected || document.visibilityState !== "visible") return;
      controller?.abort();
      controller = new AbortController();
      const now = edgeNow();
      const totalCards = queryAll("[data-live-signal]", dashboard).length;
      if (!totalCards) {
        if (state) state.textContent = "\u6709\u52B9\u306A\u8A08\u6E2C\u30EB\u30FC\u30EB\u304C\u3042\u308A\u307E\u305B\u3093\u3002\u8A08\u6E2C\u30EB\u30FC\u30EB\u3092\u8A2D\u5B9A\u3057\u3066\u304F\u3060\u3055\u3044";
        return;
      }
      const cards = activeCards(dashboard);
      if (!cards.length) {
        if (state) state.textContent = "\u8868\u793A\u9818\u57DF\u5185\u306E\u8A08\u6E2C\u30EB\u30FC\u30EB\u3092\u5F85\u3063\u3066\u3044\u307E\u3059";
        return;
      }
      for (const card of cards) {
        const cached = latestPayloads.get(card);
        if (cached) {
          renderCard(card, cached, now, staleAfterMs, sessionStartedAt);
        }
      }
      const results = await Promise.all(
        cards.map(async (card) => {
          const ruleId = card.dataset.ruleId;
          if (!ruleId) return false;
          const catchingUp = liveSnapshotAt < sessionStartedAt && !catchUpComplete.has(card);
          const requestFrom = catchingUp ? liveSnapshotAt : sessionStartedAt;
          const requestWindow = historyWindow(requestFrom, now + 1);
          const result = await getHistorySeries(
            ruleId,
            requestWindow.from,
            requestWindow.to,
            requestWindow.bucketMs,
            controller.signal
          ).catch(() => null);
          if (!result?.ok) return false;
          const payload = retainCardLatest(latestPayloads.get(card), result.value);
          const renderedPayload = catchingUp ? { ...payload, sample_count: 0, points: [] } : payload;
          if (catchingUp) catchUpComplete.add(card);
          latestPayloads.set(card, renderedPayload);
          renderCard(card, renderedPayload, now, staleAfterMs, sessionStartedAt);
          return true;
        })
      );
      if (state) {
        const succeeded = results.filter(Boolean).length;
        state.textContent = succeeded === cards.length ? `\u81EA\u52D5\u66F4\u65B0\u4E2D\u30FB${succeeded}\u4EF6\u3092\u78BA\u8A8D` : `\u4E00\u90E8\u3092\u78BA\u8A8D\u3067\u304D\u307E\u305B\u3093\u30FB${succeeded}/${cards.length}\u4EF6`;
      }
    };
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") void refresh();
      else controller?.abort();
    });
    if (document.visibilityState === "visible") void refresh();
    window.setInterval(() => void refresh(), REFRESH_MS);
  }

  // src/console.ts
  initializeShell();
  initializeLiveDashboard();
  initializeSemanticForms();
  initializePreviews();
})();
