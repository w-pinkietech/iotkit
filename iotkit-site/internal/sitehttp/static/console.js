(() => {
  const csrf = () => document.cookie.split("; ").find(v => v.startsWith("iotkit_site_csrf="))?.split("=")[1] || "";
  const number = (form, name) => Number(form.elements[name].value || 0);
  for (const form of document.querySelectorAll("form.semantic-form")) {
    const button = form.querySelector(".preview-button");
    const output = form.querySelector(".preview-output");
    let previewId = "";
    let timer = 0;
    button.addEventListener("click", async () => {
      if (previewId) {
        clearInterval(timer);
        await fetch(`/api/v1/mapping-previews/${previewId}`, {
          method: "DELETE", headers: {"X-CSRF-Token": csrf()}
        });
        previewId = "";
        button.textContent = "実信号プレビューを開始";
        return;
      }
      const body = {
        signal_ref: form.dataset.signalRef,
        spec: {
          kind: form.elements.kind.value,
          scale: number(form, "scale"),
          offset: number(form, "offset"),
          condition: {
            mode: form.elements.condition.value,
            bool_value: true,
            threshold: number(form, "threshold"),
            hysteresis: number(form, "hysteresis")
          },
          trigger: form.elements.trigger.value
        }
      };
      const response = await fetch("/api/v1/mapping-previews", {
        method: "POST",
        headers: {"Content-Type": "application/json", "X-CSRF-Token": csrf()},
        body: JSON.stringify(body)
      });
      if (!response.ok) {
        output.textContent = "プレビューを開始できませんでした。設定を確認してください。";
        return;
      }
      previewId = (await response.json()).preview_id;
      button.textContent = "プレビューを停止";
      const refresh = async () => {
        const current = await fetch(`/api/v1/mapping-previews/${previewId}`);
        if (!current.ok) {
          clearInterval(timer);
          previewId = "";
          button.textContent = "実信号プレビューを開始";
          return;
        }
        const payload = await current.json();
        output.textContent = payload.samples.length
          ? payload.samples.slice(-20).map(sample =>
              `入力 ${sample.input} → ${sample.result.emitted ? JSON.stringify(sample.result) : "変化なし"}`
            ).join("\n")
          : "開始後の新しい信号を待っています。";
      };
      await refresh();
      timer = window.setInterval(refresh, 2000);
    });
  }
})();
