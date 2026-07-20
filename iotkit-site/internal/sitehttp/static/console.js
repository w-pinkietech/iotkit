(() => {
  const csrf = () =>
    document.cookie
      .split("; ")
      .find((value) => value.startsWith("iotkit_site_csrf="))
      ?.split("=")[1] || "";

  const menuButton = document.querySelector(".menu-button");
  const overlay = document.querySelector(".mobile-overlay");
  const closeMenu = () => {
    document.body.classList.remove("menu-open");
    if (menuButton) menuButton.setAttribute("aria-expanded", "false");
    if (overlay) overlay.hidden = true;
  };
  if (menuButton && overlay) {
    menuButton.addEventListener("click", () => {
      const open = !document.body.classList.contains("menu-open");
      document.body.classList.toggle("menu-open", open);
      menuButton.setAttribute("aria-expanded", String(open));
      overlay.hidden = !open;
    });
    overlay.addEventListener("click", closeMenu);
    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape") closeMenu();
    });
  }

  const filterTable = (tableID) => {
    const table = document.getElementById(tableID);
    if (!table) return;
    const search = document.querySelector(`[data-table-search="${tableID}"]`);
    const status = document.querySelector(`[data-table-status="${tableID}"]`);
    const count = document.querySelector(`[data-table-count="${tableID}"]`);
    const apply = () => {
      const query = search?.value.trim().toLocaleLowerCase("ja") || "";
      const state = status?.value || "";
      let visible = 0;
      for (const row of table.querySelectorAll("tbody tr:not(.empty-row)")) {
        const matchesText = !query || row.textContent.toLocaleLowerCase("ja").includes(query);
        const matchesState = !state || row.dataset.status === state;
        row.hidden = !(matchesText && matchesState);
        if (!row.hidden) visible += 1;
      }
      if (count) count.textContent = String(visible);
    };
    search?.addEventListener("input", apply);
    status?.addEventListener("change", apply);
  };
  filterTable("signal-table");
  filterTable("log-table");

  document.addEventListener("click", (event) => {
    const copyButton = event.target.closest("[data-copy-text]");
    if (copyButton) {
      const originalLabel = copyButton.textContent;
      navigator.clipboard
        .writeText(copyButton.dataset.copyText || "")
        .then(() => {
          copyButton.textContent = "コピーしました";
        })
        .catch(() => {
          copyButton.textContent = "コピーできません";
        })
        .finally(() => {
          window.setTimeout(() => {
            copyButton.textContent = originalLabel;
          }, 1600);
        });
      return;
    }
    const row = event.target.closest("tr[data-href]");
    if (!row || event.target.closest("a, button, input, select, textarea")) return;
    window.location.assign(row.dataset.href);
  });

  const toggleSignalProfileFields = (form) => {
    const sensorType = form.querySelector("[data-sensor-type]");
    const customLabel = form.querySelector("[data-custom-sensor-label]");
    const valueKind = form.querySelector("[data-value-kind]");
    const unitMode = form.querySelector("[data-unit-mode]");
    const unitField = form.querySelector("[data-display-unit]");
    const decimalField = form.querySelector("[data-decimal-places]");
    const update = () => {
      if (customLabel) {
        customLabel.hidden = sensorType?.value !== "custom";
        const input = customLabel.querySelector("input");
        if (input) input.required = sensorType?.value === "custom";
      }
      if (valueKind?.value === "boolean") {
        if (unitMode) unitMode.value = "dimensionless";
        const unitInput = unitField?.querySelector("input");
        if (unitInput) unitInput.value = "";
        const decimalInput = decimalField?.querySelector("input");
        if (decimalInput) decimalInput.value = "0";
      }
      const hasUnit = unitMode?.value === "unit" && valueKind?.value !== "boolean";
      if (unitField) unitField.hidden = !hasUnit;
      const unitInput = unitField?.querySelector("input");
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
  };
  for (const form of document.querySelectorAll("form[data-signal-profile]")) {
    toggleSignalProfileFields(form);
  }

  for (const form of document.querySelectorAll("[data-output-route-form]")) {
    const rule = form.querySelector("[data-output-rule]");
    const adapter = form.querySelector("[data-output-adapter]");
    const mode = form.querySelector("[data-output-mode]");
    const sourceID = form.querySelector('[name="source_id"]');
    const signalID = form.querySelector('[name="signal_id"]');
    const yokakitTopicPreview = form.querySelector(
      "[data-yokakit-topic-preview]",
    );
    const payloadPreview = form.querySelector("[data-output-payload-preview]");
    const yokakitPayloadPreview = form.querySelector(
      "[data-yokakit-payload-preview]",
    );
    const payloadExamples = {
      numeric: '{"schema_version":1,"kind":"numeric","value":24.8}',
      boolean: '{"schema_version":1,"kind":"boolean","value":true}',
      cumulative_value:
        '{"schema_version":1,"kind":"cumulative_value","value":1524}',
      alarm: '{"schema_version":1,"kind":"alarm","value":true}',
    };
    const yokakitPayloadExamples = {
      production: '{"schema_version":1,"kind":"production","value":1524}',
      onoff: '{"schema_version":1,"kind":"onoff","value":true}',
      gantt_chart: '{"schema_version":1,"kind":"gantt_chart","value":true}',
      alarm:
        '{"schema_version":1,"kind":"alarm","value":true,"reason":"温度上限を超過"}',
    };
    const supports = (option, kind) => {
      if (!kind) return true;
      return (option?.dataset.accepts || "").split(/\s+/).includes(kind);
    };
    const update = () => {
      const kind = rule?.selectedOptions[0]?.dataset.kind || "";
      if (payloadPreview) {
        payloadPreview.textContent =
          payloadExamples[kind] ||
          "値を選ぶと、ここにpayloadの例を表示します";
      }
      if (adapter) {
        for (const option of adapter.options) {
          option.disabled = !supports(option, kind);
        }
        if (adapter.selectedOptions[0]?.disabled) {
          const compatible = Array.from(adapter.options).find((option) => !option.disabled);
          if (compatible) adapter.value = compatible.value;
        }
      }
      for (const fields of form.querySelectorAll("[data-output-fields]")) {
        const active = fields.dataset.outputFields === adapter?.value;
        fields.hidden = !active;
        for (const input of fields.querySelectorAll("input, select, textarea")) {
          input.disabled = !active;
          if (input.name === "topic" || input.name === "source_id" ||
              input.name === "signal_id") {
            input.required = active;
          }
        }
      }
      if (mode) {
        for (const option of mode.options) {
          option.disabled = !supports(option, kind);
          option.hidden = option.disabled;
        }
        if (mode.selectedOptions[0]?.disabled) {
          const compatible = Array.from(mode.options).find((option) => !option.disabled);
          if (compatible) mode.value = compatible.value;
        }
      }
      if (yokakitPayloadPreview) {
        yokakitPayloadPreview.textContent =
          yokakitPayloadExamples[mode?.value] ||
          "用途を選ぶと、ここにpayloadの例を表示します";
      }
      if (yokakitTopicPreview) {
        const source = sourceID?.value || "{source-id}";
        const signal = signalID?.value || "{signal-id}";
        yokakitTopicPreview.textContent =
          `yokakit/v1/sources/${source}/signals/${signal}/observations`;
      }
    };
    rule?.addEventListener("change", update);
    adapter?.addEventListener("change", update);
    mode?.addEventListener("change", update);
    sourceID?.addEventListener("input", update);
    signalID?.addEventListener("input", update);
    update();
  }

  const toggleSemanticFields = (form) => {
    const kind = form.querySelector("[data-semantic-kind]");
    const detectorFields = form.querySelector("[data-semantic-detector]");
    const detector = form.elements.detector_mode;
    const thresholds = form.querySelector("[data-semantic-thresholds]");
    const triggerFields = form.querySelector("[data-semantic-trigger]");
    const trigger = form.elements.trigger;
    const booleanInput = form.dataset.booleanInput === "true";
    if (!kind || !detectorFields || !detector || !triggerFields || !trigger) return;
    const toggleFields = (container, visible) => {
      if (!container) return;
      container.hidden = !visible;
      container.querySelectorAll("input, select").forEach((field) => {
        field.disabled = !visible;
      });
    };
    const update = () => {
      const selectedKind = kind.value;
      const needsDetector = selectedKind !== "numeric";
      const countsValues = selectedKind === "cumulative_counter";
      toggleFields(detectorFields, needsDetector);
      toggleFields(triggerFields, countsValues);
      for (const option of detector.options) {
        const matchesInput = booleanInput
          ? option.hasAttribute("data-detector-boolean")
          : option.hasAttribute("data-detector-analog");
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
  };

  const number = (form, name) => Number(form.elements[name]?.value || 0);
  for (const form of document.querySelectorAll("form.semantic-form")) {
    toggleSemanticFields(form);
  }
  const semanticRuleCards = Array.from(
    document.querySelectorAll("details.semantic-rule-card"),
  );
  if (semanticRuleCards.length) semanticRuleCards[0].open = true;
  for (const card of semanticRuleCards) {
    card.addEventListener("toggle", () => {
      if (!card.open) return;
      for (const other of semanticRuleCards) {
        if (other !== card) other.open = false;
      }
    });
  }

  const svgNS = "http://www.w3.org/2000/svg";
  const addSVG = (parent, name, attributes = {}) => {
    const element = document.createElementNS(svgNS, name);
    for (const [key, value] of Object.entries(attributes)) {
      element.setAttribute(key, String(value));
    }
    parent.appendChild(element);
    return element;
  };
  const finite = (value) => Number.isFinite(Number(value));
  const formatNumber = (value) =>
    Number(value).toLocaleString("ja-JP", {maximumFractionDigits: 3});
  const formatPreviewCurrentValue = (value, valueKind) =>
    valueKind === "boolean" ? (Number(value) === 0 ? "OFF" : "ON") : formatNumber(value);
  const formatDuration = (start, end) => {
    const milliseconds = Math.max(0, Number(end) - Number(start));
    if (milliseconds < 60_000) return `${Math.max(1, Math.round(milliseconds / 1000))}秒`;
    return `${Math.max(1, Math.round(milliseconds / 60_000))}分`;
  };
  const semanticSpec = (form) => ({
    kind: form.elements.kind.value,
    scale: number(form, "scale"),
    offset: number(form, "offset"),
    detector: {
      mode: form.elements.detector_mode?.value || "",
      rise_threshold: number(form, "rise_threshold"),
      fall_threshold: number(form, "fall_threshold"),
      rise_debounce_ms: Math.round(number(form, "rise_debounce_seconds") * 1000),
      fall_debounce_ms: Math.round(number(form, "fall_debounce_seconds") * 1000),
    },
    trigger: form.elements.trigger?.value || "",
  });
  const semanticRuleSpec = (form) => ({
    kind: form.elements.kind.value,
    detector: {
      mode: form.elements.detector_mode?.value || "",
      rise_threshold: number(form, "rise_threshold"),
      fall_threshold: number(form, "fall_threshold"),
      rise_debounce_ms: Math.round(number(form, "rise_debounce_seconds") * 1000),
      fall_debounce_ms: Math.round(number(form, "fall_debounce_seconds") * 1000),
    },
    trigger: form.elements.trigger?.value || "",
  });

  const renderPreviewChart = (svg, payload) => {
    svg.replaceChildren();
    const points = payload.points || [];
    const width = 760;
    const height = 260;
    const left = 58;
    const right = 18;
    const top = 18;
    const bottom = 42;
    const plotWidth = width - left - right;
    const plotHeight = height - top - bottom;
    if (!points.length) {
      const empty = addSVG(svg, "text", {
        x: width / 2,
        y: height / 2 - 8,
        "text-anchor": "middle",
        class: "chart-empty-title",
      });
      empty.textContent = payload.error
        ? "このルールでは受信値を判定できません"
        : "まだ受信データがありません";
      const hint = addSVG(svg, "text", {
        x: width / 2,
        y: height / 2 + 18,
        "text-anchor": "middle",
        class: "chart-empty-hint",
      });
      hint.textContent = payload.error
        ? "入力値の補正と判定条件を確認してください"
        : "試す値を入力して、設定結果を確認できます";
      return;
    }

    const values = [];
    for (const point of points) {
      for (const value of [
        point.input_min,
        point.input_max,
        point.calibrated_min,
        point.calibrated_max,
      ]) {
        if (finite(value)) values.push(Number(value));
      }
    }
    if (finite(payload.rise_threshold)) values.push(Number(payload.rise_threshold));
    if (finite(payload.fall_threshold)) values.push(Number(payload.fall_threshold));
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
    const firstReceivedAt = Number(points[0].received_at);
    const lastReceivedAt = Number(points.at(-1).received_at);
    const x = (index) => {
      if (points.length === 1) return left + plotWidth / 2;
      const point = points[index];
      if (lastReceivedAt > firstReceivedAt) {
        return (
          left +
          ((Number(point.received_at) - firstReceivedAt) * plotWidth) /
            (lastReceivedAt - firstReceivedAt)
        );
      }
      return left + (index * plotWidth) / (points.length - 1);
    };
    const y = (value) =>
      top + ((maxValue - Number(value)) * plotHeight) / (maxValue - minValue);

    for (let index = 0; index <= 4; index += 1) {
      const gridY = top + (index * plotHeight) / 4;
      addSVG(svg, "line", {
        x1: left,
        x2: width - right,
        y1: gridY,
        y2: gridY,
        class: "chart-grid",
      });
      const label = addSVG(svg, "text", {
        x: left - 9,
        y: gridY + 4,
        "text-anchor": "end",
        class: "chart-axis-label",
      });
      label.textContent = formatNumber(maxValue - (index * (maxValue - minValue)) / 4);
    }

    const drawThreshold = (value, labelText) => {
      if (!finite(value)) return;
      const thresholdY = y(value);
      addSVG(svg, "line", {
        x1: left,
        x2: width - right,
        y1: thresholdY,
        y2: thresholdY,
        class: "chart-threshold",
      });
      const label = addSVG(svg, "text", {
        x: width - right - 4,
        y: thresholdY - 6,
        "text-anchor": "end",
        class: "chart-threshold-label",
      });
      label.textContent = `${labelText} ${formatNumber(value)}`;
    };
    drawThreshold(payload.rise_threshold, "立上り");
    drawThreshold(payload.fall_threshold, "立下り");

    points.forEach((point, index) => {
      if (point.sample_count > 1) {
        addSVG(svg, "line", {
          x1: x(index),
          x2: x(index),
          y1: y(point.input_min),
          y2: y(point.input_max),
          class: "chart-range",
        });
        addSVG(svg, "line", {
          x1: x(index) + 2,
          x2: x(index) + 2,
          y1: y(point.calibrated_min),
          y2: y(point.calibrated_max),
          class: "chart-range-result",
        });
      }
      if (payload.kind !== "numeric") {
        const ratio = point.sample_count
          ? Number(point.active_samples || 0) / Number(point.sample_count)
          : 0;
        if (ratio > 0) {
          addSVG(svg, "rect", {
            x: x(index) - Math.max(1, plotWidth / Math.max(points.length, 1) / 2),
            y: top,
            width: Math.max(2, plotWidth / Math.max(points.length, 1)),
            height: plotHeight,
            class: "chart-active-band",
            opacity: Math.max(0.12, ratio * 0.24),
          });
        }
      }
    });

    const path = (field) =>
      points
        .map((point, index) =>
          `${index === 0 ? "M" : "L"} ${x(index).toFixed(2)} ${y(point[field]).toFixed(2)}`,
        )
        .join(" ");
    addSVG(svg, "path", {d: path("input"), class: "chart-line chart-line-raw"});
    addSVG(svg, "path", {
      d: path("calibrated"),
      class: "chart-line chart-line-result",
    });

    if (payload.kind === "cumulative_counter") {
      const maxIncrement = Math.max(1, ...points.map((point) => Number(point.increment || 0)));
      points.forEach((point, index) => {
        const increment = Number(point.increment || 0);
        if (!increment) return;
        const barHeight = Math.max(3, (increment / maxIncrement) * 34);
        addSVG(svg, "rect", {
          x: x(index) - 2,
          y: top + plotHeight - barHeight,
          width: 4,
          height: barHeight,
          class: "chart-increment",
        });
      });
      const maxCounter = Math.max(1, ...points.map((point) => Number(point.counter || 0)));
      const counterY = (value) =>
        top + ((maxCounter - Number(value || 0)) * plotHeight) / maxCounter;
      const counterPath = points
        .map(
          (point, index) =>
            `${index === 0 ? "M" : "L"} ${x(index).toFixed(2)} ` +
            `${counterY(point.counter).toFixed(2)}`,
        )
        .join(" ");
      addSVG(svg, "path", {
        d: counterPath,
        class: "chart-line chart-line-counter",
      });
      const counterLabel = addSVG(svg, "text", {
        x: width - right - 4,
        y: counterY(points.at(-1)?.counter) - 7,
        "text-anchor": "end",
        class: "chart-counter-label",
      });
      counterLabel.textContent = `累積 ${formatNumber(points.at(-1)?.counter || 0)}`;
    }

    const start = addSVG(svg, "text", {
      x: left,
      y: height - 14,
      class: "chart-axis-label",
    });
    start.textContent = new Date(payload.window_start).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
    const end = addSVG(svg, "text", {
      x: width - right,
      y: height - 14,
      "text-anchor": "end",
      class: "chart-axis-label",
    });
    end.textContent = new Date(payload.window_end).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  };

  for (const panel of document.querySelectorAll("[data-setting-simulation]")) {
    const signalRef = panel.dataset.signalRef;
    const forms = Array.from(
      document.querySelectorAll(`form.semantic-form[data-signal-ref="${signalRef}"]`),
    );
    const form = forms[0];
    const calibrationForm = document.querySelector(
      `form[action="/console/signals/${signalRef}/calibration"]`,
    );
    const ruleCards = forms
      .map((candidate) => candidate.closest("details.semantic-rule-card"))
      .filter(Boolean);
    const usesMultipleRules =
      forms.some((candidate) => candidate.dataset.ruleId) ||
      forms.some((candidate) => candidate.action.endsWith("/semantic-rules"));
    const testInput = panel.querySelector("[name=preview_test_value]");
    const testResult = panel.querySelector("[data-preview-test-result]");
    const range = panel.querySelector("[data-preview-range]");
    const count = panel.querySelector("[data-preview-count]");
    const message = panel.querySelector("[data-preview-message]");
    const chart = panel.querySelector("[data-preview-chart]");
    const currentValue = panel.querySelector("[data-preview-current-value]");
    const currentReceived = panel.querySelector("[data-preview-current-received]");
    const sourceFlow = document.querySelector(".sensor-flow-input [data-source-value]");
    const sourceCurrentValue = sourceFlow?.querySelector("[data-source-current-value]");
    const sourceCurrentReceived = sourceFlow?.querySelector(
      "[data-source-current-received]",
    );
    const valueKind = document.querySelector(
      'form[data-signal-profile] [name="display_value_kind"]',
    );
    let controller;
    let debounce = 0;
    let previewUnavailable = false;

    const refresh = async () => {
      controller?.abort();
      controller = new AbortController();
      form?.querySelectorAll('[aria-invalid="true"]').forEach((field) => {
        field.removeAttribute("aria-invalid");
      });
      const body = {signal_ref: signalRef};
      if (usesMultipleRules) {
        body.calibration = {
          scale: calibrationForm ? number(calibrationForm, "scale") : 1,
          offset: number(calibrationForm, "offset"),
        };
        body.rules = forms
          .filter((candidate) =>
            candidate.dataset.ruleId ||
            candidate.elements.display_name?.value.trim(),
          )
          .map((candidate, index) => ({
            rule_id: candidate.dataset.ruleId || `draft-${index + 1}`,
            display_name:
              candidate.elements.display_name?.value.trim() || `ルール ${index + 1}`,
            spec: semanticRuleSpec(candidate),
          }));
        if (!body.rules.length) {
          delete body.calibration;
          delete body.rules;
        }
      } else if (form) {
        body.spec = semanticSpec(form);
      }
      if (testInput?.value.trim()) body.test_value = Number(testInput.value);
      try {
        const response = await fetch("/api/v1/mapping-previews", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-CSRF-Token": csrf(),
          },
          body: JSON.stringify(body),
          signal: controller.signal,
        });
        if (!response.ok) {
          const failure = await response.json().catch(() => ({}));
          const invalidField = failure.error?.field
            ? form?.elements[failure.error.field]
            : null;
          invalidField?.setAttribute("aria-invalid", "true");
          const fieldLabel = invalidField
            ?.closest("label")
            ?.querySelector(":scope > span")
            ?.textContent?.trim();
          if (response.status === 404 && !form) {
            previewUnavailable = true;
            message.textContent = "値の変換が設定されると、ここに設定結果を表示します。";
          } else if (fieldLabel) {
            message.textContent = `${fieldLabel}を確認してください。最後に確認できたグラフを表示しています。`;
          } else {
            message.textContent = "設定内容を確認してください。最後に確認できたグラフを表示しています。";
          }
          return;
        }
        const responsePayload = await response.json();
        const selectedRuleID = ruleCards.find((card) => card.open)?.dataset.ruleId;
        const firstRule =
          responsePayload.rules?.find(
            (rule) => rule.rule_id === selectedRuleID,
          ) ||
          responsePayload.rules?.find((rule) => !rule.error) ||
          responsePayload.rules?.[0];
        const payload = firstRule
          ? {
              ...firstRule,
              window_start: responsePayload.window_start,
              window_end: responsePayload.window_end,
              truncated_by: responsePayload.truncated_by,
            }
          : responsePayload;
        renderPreviewChart(chart, payload);
        const latest = payload.points?.at(-1);
        if (latest && currentValue) {
          currentValue.textContent = formatPreviewCurrentValue(latest.input, valueKind?.value);
        }
        if (latest && sourceCurrentValue) {
          const rawValue = formatNumber(latest.input);
          sourceCurrentValue.textContent = rawValue;
          sourceFlow.dataset.sourceValue = rawValue;
        }
        if (latest && (currentReceived || sourceCurrentReceived)) {
          const elapsed = Math.max(0, Date.now() - Number(latest.received_at));
          const relative =
            elapsed < 5_000
              ? "たった今"
              : elapsed < 60_000
                ? `${Math.floor(elapsed / 1_000)}秒前`
                : `${Math.floor(elapsed / 60_000)}分前`;
          const receivedTitle = new Date(latest.received_at).toLocaleString("ja-JP");
          if (currentReceived) {
            currentReceived.textContent = `最終受信 ${relative}`;
            currentReceived.title = receivedTitle;
          }
          if (sourceCurrentReceived) {
            sourceCurrentReceived.textContent = relative;
            sourceCurrentReceived.title = receivedTitle;
          }
        }
        if (payload.input_count === 0) {
          range.textContent = "受信データはまだありません";
          count.textContent = "試す値で設定結果を確認できます。";
          message.textContent = "履歴は作らず、実際に届いた値だけを表示します。";
        } else {
          const duration = formatDuration(payload.window_start, payload.window_end);
          range.textContent = `直近${duration}の受信値`;
          count.textContent =
            `${Number(payload.input_count).toLocaleString("ja-JP")}件を` +
            `${Number(payload.plot_count).toLocaleString("ja-JP")}点で表示`;
          message.textContent =
            payload.truncated_by === "input_count"
              ? "高速な信号のため、最新20,000件を要約しています。"
              : payload.kind === "cumulative_counter"
                ? `表示範囲内の累積値は ${
                    payload.points.at(-1)?.counter ?? 0
                  } です。先頭の値は数えません。`
                : "設定を変えると、保存前の結果をこのグラフで確認できます。";
        }
        if (testResult) {
          const result = payload.test_result;
          if (!result) {
            testResult.textContent = "値を入力すると結果を確認できます";
          } else if (result.number !== undefined) {
            testResult.textContent = formatNumber(result.number);
          } else if (result.boolean !== undefined) {
            testResult.textContent = result.boolean ? "ON" : "OFF";
          } else if (result.integer !== undefined) {
            testResult.textContent = `累積 ${formatNumber(result.integer)}`;
          } else if (payload.kind === "cumulative_counter") {
            testResult.textContent = "最初の値として確認（累積には加えません）";
          } else {
            testResult.textContent = `補正後 ${formatNumber(result.calibrated)}`;
          }
        }
      } catch (error) {
        if (error.name !== "AbortError") {
          message.textContent = "設定結果を更新できません。データ受信には影響ありません。";
        }
      }
    };

    const schedule = () => {
      window.clearTimeout(debounce);
      debounce = window.setTimeout(refresh, 300);
    };
    for (const candidate of forms) {
      candidate.addEventListener("input", schedule);
      candidate.addEventListener("change", schedule);
    }
    calibrationForm?.addEventListener("input", schedule);
    calibrationForm?.addEventListener("change", schedule);
    for (const card of ruleCards) {
      card.addEventListener("toggle", () => {
        if (card.open) schedule();
      });
    }
    testInput?.addEventListener("input", schedule);
    refresh();
    window.setInterval(() => {
      if (document.visibilityState === "visible" && !previewUnavailable) refresh();
    }, 1000);
  }
})();
