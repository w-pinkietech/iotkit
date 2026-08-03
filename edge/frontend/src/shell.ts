import { query, queryAll } from "./dom";

export const SETTING_TAB_CHANGE_EVENT = "iotkit:setting-tab-change";

export function csrfToken(): string {
  const value = document.cookie
    .split("; ")
    .find((cookie) => cookie.startsWith("iotkit_edge_csrf="));
  return value?.split("=")[1] ?? "";
}

function initializeMenu(): void {
  const menuButton = query<HTMLButtonElement>(".menu-button");
  const overlay = query<HTMLButtonElement>(".mobile-overlay");
  const sidebar = query<HTMLElement>(".sidebar");
  if (!menuButton || !overlay || !sidebar) return;

  const compactLayout = window.matchMedia("(max-width: 960px)");
  const setOpen = (open: boolean, restoreFocus = false): void => {
    document.body.classList.toggle("menu-open", open);
    menuButton.setAttribute("aria-expanded", String(open));
    overlay.hidden = !open;

    if (open) {
      (
        query<HTMLAnchorElement>(".side-nav a.active", sidebar) ??
        query<HTMLAnchorElement>(".side-nav a", sidebar)
      )?.focus();
    } else if (restoreFocus) {
      menuButton.focus();
    }
  };

  menuButton.addEventListener("click", () => {
    const open = !document.body.classList.contains("menu-open");
    setOpen(open);
  });
  overlay.addEventListener("click", () => setOpen(false, true));
  for (const link of queryAll<HTMLAnchorElement>(".side-nav a", sidebar)) {
    link.addEventListener("click", () => setOpen(false));
  }
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") setOpen(false, true);
  });
  compactLayout.addEventListener("change", (event) => {
    if (!event.matches) setOpen(false);
  });
}

function initializeTableFilter(tableID: string): void {
  const table = document.getElementById(tableID);
  if (!(table instanceof HTMLTableElement)) return;

  const search = query<HTMLInputElement>(
    `[data-table-search="${tableID}"]`,
  );
  const status = query<HTMLSelectElement>(
    `[data-table-status="${tableID}"]`,
  );
  const count = query<HTMLElement>(`[data-table-count="${tableID}"]`);
  const apply = (): void => {
    const searchText = search?.value.trim().toLocaleLowerCase("ja") ?? "";
    const selectedState = status?.value ?? "";
    let visible = 0;
    for (const row of queryAll<HTMLTableRowElement>(
      "tbody tr:not(.empty-row)",
      table,
    )) {
      const rowText = row.textContent?.toLocaleLowerCase("ja") ?? "";
      const matchesText = !searchText || rowText.includes(searchText);
      const matchesState =
        !selectedState || row.dataset.status === selectedState;
      row.hidden = !(matchesText && matchesState);
      if (!row.hidden) visible += 1;
    }
    if (count) count.textContent = String(visible);
  };
  search?.addEventListener("input", apply);
  status?.addEventListener("change", apply);
}

function initializeDocumentActions(): void {
  for (const form of queryAll<HTMLFormElement>("form[data-confirm-message]")) {
    form.addEventListener("submit", (event) => {
      const message = form.dataset.confirmMessage;
      if (message && !window.confirm(message)) {
        event.preventDefault();
      }
    });
  }

  document.addEventListener("click", (event) => {
    if (!(event.target instanceof Element)) return;

    const copyButton = event.target.closest<HTMLElement>("[data-copy-text]");
    if (copyButton) {
      const originalLabel = copyButton.textContent;
      navigator.clipboard
        .writeText(copyButton.dataset.copyText ?? "")
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

    const row = event.target.closest<HTMLTableRowElement>("tr[data-href]");
    if (
      !row ||
      event.target.closest("a, button, input, select, textarea") ||
      !row.dataset.href
    ) {
      return;
    }
    window.location.assign(row.dataset.href);
  });
}

function initializeSettingTabs(): void {
  for (const root of queryAll<HTMLElement>("[data-setting-tabs]")) {
    const tabs = queryAll<HTMLButtonElement>("[data-setting-tab]", root);
    const panels = queryAll<HTMLElement>("[data-setting-panel]", root);
    if (!tabs.length || !panels.length) continue;

    const activate = (
      key: string,
      focus = false,
      replaceTabQuery = false,
    ): void => {
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
        new CustomEvent<{ key: string }>(SETTING_TAB_CHANGE_EVENT, {
          detail: { key },
          bubbles: true,
        }),
      );
    };

    let initial = root.dataset.defaultSettingTab ?? tabs[0].dataset.settingTab;
    const focusedID = document.body.dataset.focusTarget;
    const focused = focusedID ? document.getElementById(focusedID) : null;
    const focusedPanel =
      focused && root.contains(focused)
        ? focused.closest<HTMLElement>("[data-setting-panel]")
        : null;
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

function initializeFocusedSection(): void {
  const targetID = document.body.dataset.focusTarget;
  if (!targetID) return;
  const target = document.getElementById(targetID);
  if (!target) return;
  if (target instanceof HTMLDetailsElement) {
    target.open = true;
    target.querySelector<HTMLElement>("summary")?.focus();
    return;
  }
  target.setAttribute("tabindex", "-1");
  target.focus();
}

function initializeLocalizedTimes(): void {
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
    timeZoneName: "short",
  });
  for (const timestamp of queryAll<HTMLElement | SVGTextElement>("[data-unix-ms]")) {
    const milliseconds = Number(timestamp.dataset.unixMs);
    if (!Number.isFinite(milliseconds)) continue;
    const date = new Date(milliseconds);
    if (Number.isNaN(date.getTime())) continue;
    if (timestamp instanceof HTMLTimeElement) {
      timestamp.dateTime = date.toISOString();
    }
    timestamp.textContent = formatter.format(date);
  }
  const checkedAt = query<HTMLTimeElement>("[data-activation-checked-at]");
  if (checkedAt) {
    const now = new Date();
    checkedAt.dateTime = now.toISOString();
    checkedAt.textContent = now.toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }
}

function initializeActivationRefresh(reload: () => void): void {
  const key = `iotkit-activation-refresh:${window.location.pathname}`;
  if (document.body.dataset.activationRefresh !== "true") {
    sessionStorage.removeItem(key);
    return;
  }
  const checkNow = query<HTMLButtonElement>("[data-activation-check-now]");
  if (checkNow && checkNow.dataset.activationBound !== "true") {
    checkNow.dataset.activationBound = "true";
    checkNow.addEventListener("click", () => {
      sessionStorage.setItem(key, "0");
      reload();
    });
  }
  const attempts = Number(sessionStorage.getItem(key) ?? "0");
  if (!Number.isFinite(attempts) || attempts >= 20) {
    const state = query<HTMLElement>("[data-activation-state]");
    const guidance = query<HTMLElement>("[data-activation-guidance]");
    if (state) state.textContent = "自動確認を一時停止しました";
    if (guidance) {
      guidance.textContent =
        "自動確認の上限に達したため一時停止しました。" +
        "サーバー側の登録処理は続いています。" +
        "「今すぐ確認」で確認を再開できます。";
    }
    return;
  }
  window.setTimeout(() => {
    sessionStorage.setItem(key, String(attempts + 1));
    reload();
  }, 3_000);
}

function initializeSignalProfile(form: HTMLFormElement): void {
  const sensorType = query<HTMLSelectElement>("[data-sensor-type]", form);
  const customLabel = query<HTMLElement>("[data-custom-sensor-label]", form);
  const valueKind = query<HTMLSelectElement>("[data-value-kind]", form);
  const unitMode = query<HTMLSelectElement>("[data-unit-mode]", form);
  const unitField = query<HTMLElement>("[data-display-unit]", form);
  const decimalField = query<HTMLElement>("[data-decimal-places]", form);

  const update = (): void => {
    if (customLabel) {
      const usesCustomLabel = sensorType?.value === "custom";
      customLabel.hidden = !usesCustomLabel;
      const input = query<HTMLInputElement>("input", customLabel);
      if (input) input.required = usesCustomLabel;
    }
    if (valueKind?.value === "boolean") {
      if (unitMode) unitMode.value = "dimensionless";
      const unitInput = unitField
        ? query<HTMLInputElement>("input", unitField)
        : null;
      if (unitInput) unitInput.value = "";
      const decimalInput = decimalField
        ? query<HTMLInputElement>("input", decimalField)
        : null;
      if (decimalInput) decimalInput.value = "0";
    }
    const hasUnit =
      unitMode?.value === "unit" && valueKind?.value !== "boolean";
    if (unitField) unitField.hidden = !hasUnit;
    const unitInput = unitField
      ? query<HTMLInputElement>("input", unitField)
      : null;
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

export function initializeShell(
  reload: () => void = () => window.location.reload(),
): void {
  initializeMenu();
  initializeTableFilter("signal-table");
  initializeTableFilter("log-table");
  initializeDocumentActions();
  initializeLocalizedTimes();
  initializeActivationRefresh(reload);
  initializeSettingTabs();
  for (const form of queryAll<HTMLFormElement>("form[data-signal-profile]")) {
    initializeSignalProfile(form);
  }
  initializeFocusedSection();
}
