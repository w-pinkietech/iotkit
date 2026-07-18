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

  const toggleSemanticFields = (form) => {
    const kind = form.querySelector("[data-semantic-kind]");
    const fields = form.querySelector("[data-condition-fields]");
    if (!kind || !fields) return;
    const update = () => {
      const needsCondition = kind.value !== "numeric";
      fields.hidden = !needsCondition;
      fields.querySelectorAll("input, select").forEach((field) => {
        field.disabled = !needsCondition;
      });
    };
    kind.addEventListener("change", update);
    update();
  };

  const number = (form, name) => Number(form.elements[name]?.value || 0);
  for (const form of document.querySelectorAll("form.semantic-form")) {
    toggleSemanticFields(form);
    const button = form.querySelector(".preview-button");
    const output = form.querySelector(".preview-output");
    if (!button || !output) continue;
    let previewID = "";
    let timer = 0;

    const stop = async () => {
      window.clearInterval(timer);
      if (previewID) {
        await fetch(`/api/v1/mapping-previews/${previewID}`, {
          method: "DELETE",
          headers: {"X-CSRF-Token": csrf()},
        });
      }
      previewID = "";
      button.textContent = "実信号で確認する";
    };

    button.addEventListener("click", async () => {
      if (previewID) {
        await stop();
        output.textContent = "プレビューを停止しました。";
        return;
      }
      const body = {
        signal_ref: form.dataset.signalRef,
        spec: {
          kind: form.elements.kind.value,
          scale: number(form, "scale"),
          offset: number(form, "offset"),
          condition: {
            mode: form.elements.condition?.value || "",
            bool_value: form.elements.bool_value?.value !== "false",
            threshold: number(form, "threshold"),
            hysteresis: number(form, "hysteresis"),
          },
          trigger: form.elements.trigger?.value || "",
        },
      };
      const response = await fetch("/api/v1/mapping-previews", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-CSRF-Token": csrf(),
        },
        body: JSON.stringify(body),
      });
      if (!response.ok) {
        output.textContent = "プレビューを開始できませんでした。入力内容を確認してください。";
        return;
      }
      previewID = (await response.json()).preview_id;
      button.textContent = "プレビューを停止";

      const refresh = async () => {
        const current = await fetch(`/api/v1/mapping-previews/${previewID}`);
        if (!current.ok) {
          await stop();
          output.textContent = "プレビューを継続できませんでした。もう一度開始してください。";
          return;
        }
        const payload = await current.json();
        output.textContent = payload.samples.length
          ? payload.samples
              .slice(-8)
              .map((sample) =>
                `入力 ${sample.input} → ${
                  sample.result.emitted ? JSON.stringify(sample.result) : "変化なし"
                }`,
              )
              .join("\n")
          : "新しい信号を待っています。センサーを動かすと、ここに結果が表示されます。";
      };
      await refresh();
      timer = window.setInterval(refresh, 2000);
    });
  }
})();
