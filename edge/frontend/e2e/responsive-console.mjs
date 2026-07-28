export const responsiveConsolePaths = [
  "/status",
  "/sensors",
  "/logs",
  "/equipment",
  "/output",
  "/audit",
  "/accounts",
  "/system",
];

const responsiveStateExpression = `(() => {
  const viewportWidth = document.documentElement.clientWidth;
  const visible = (element) => {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.display !== "none" &&
      style.visibility !== "hidden" &&
      rect.width > 0 &&
      rect.height > 0;
  };
  const selectors = [
    "main",
    ".page-header",
    ".content-card",
    ".metric-card",
    ".equipment-overview",
    ".equipment-row",
    ".toolbar",
    ".status-signal-table tr",
    ".account-table tr",
    "main button",
    "main input",
    "main select",
    "main textarea",
  ];
  const clipped = [...document.querySelectorAll(selectors.join(","))]
    .filter((element) => !element.closest(".table-wrap"))
    .filter(visible)
    .map((element) => ({
      element: element.id
        ? "#" + element.id
        : element.classList.length
          ? "." + [...element.classList].join(".")
          : element.tagName.toLowerCase(),
      rect: element.getBoundingClientRect(),
    }))
    .filter(({ rect }) => rect.left < -0.5 || rect.right > viewportWidth + 0.5)
    .map(({ element, rect }) => ({
      element,
      left: Math.round(rect.left),
      right: Math.round(rect.right),
    }));
  const denseTables = ["#log-table", "#audit-table"]
    .map((selector) => document.querySelector(selector))
    .filter(Boolean);
  const uncontainedTables = denseTables
    .filter((table) => {
      const wrapper = table.closest(".table-wrap");
      return !wrapper || !["auto", "scroll"].includes(getComputedStyle(wrapper).overflowX);
    })
    .map((table) => "#" + table.id);
  return {
    documentFits: document.documentElement.scrollWidth <= viewportWidth,
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth,
    clipped,
    uncontainedTables,
  };
})()`;

function assertResponsive(path, width, state) {
  if (
    !state.documentFits ||
    state.clipped.length > 0 ||
    state.uncontainedTables.length > 0
  ) {
    throw new Error(
      `${path} is not responsive at ${width}px: ${JSON.stringify(state)}`,
    );
  }
}

async function verifyDrawer({ devtools, navigate }) {
  await devtools.send("Emulation.setDeviceMetricsOverride", {
    width: 390,
    height: 844,
    deviceScaleFactor: 1,
    mobile: true,
  });
  await navigate("/status");

  const opened = await devtools.evaluate(`(() => {
    const button = document.querySelector(".menu-button");
    button.click();
    return {
      expanded: button.getAttribute("aria-expanded"),
      open: document.body.classList.contains("menu-open"),
      overlayVisible: !document.querySelector(".mobile-overlay").hidden,
      focusedActiveLink: document.activeElement ===
        document.querySelector(".side-nav a.active"),
    };
  })()`);
  if (
    opened.expanded !== "true" ||
    !opened.open ||
    !opened.overlayVisible ||
    !opened.focusedActiveLink
  ) {
    throw new Error(`mobile drawer did not open: ${JSON.stringify(opened)}`);
  }

  const escaped = await devtools.evaluate(`(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    const button = document.querySelector(".menu-button");
    return {
      closed: !document.body.classList.contains("menu-open"),
      collapsed: button.getAttribute("aria-expanded") === "false",
      focusReturned: document.activeElement === button,
    };
  })()`);
  if (!escaped.closed || !escaped.collapsed || !escaped.focusReturned) {
    throw new Error(
      `mobile drawer did not close for Escape: ${JSON.stringify(escaped)}`,
    );
  }

  const overlayClosed = await devtools.evaluate(`(() => {
    const button = document.querySelector(".menu-button");
    button.click();
    document.querySelector(".mobile-overlay").click();
    return {
      closed: !document.body.classList.contains("menu-open"),
      focusReturned: document.activeElement === button,
    };
  })()`);
  if (!overlayClosed.closed || !overlayClosed.focusReturned) {
    throw new Error(
      `mobile drawer overlay did not close: ${JSON.stringify(overlayClosed)}`,
    );
  }

  const navigationClosed = await devtools.evaluate(`(() => {
    const button = document.querySelector(".menu-button");
    const link = document.querySelector(".side-nav a[href='/equipment']");
    link.addEventListener("click", (event) => event.preventDefault(), {
      once: true,
    });
    button.click();
    link.click();
    return !document.body.classList.contains("menu-open");
  })()`);
  if (!navigationClosed) {
    throw new Error("mobile drawer did not close for navigation");
  }

  await devtools.evaluate(`document.querySelector(".menu-button").click()`);
  await devtools.send("Emulation.setDeviceMetricsOverride", {
    width: 1024,
    height: 844,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await devtools.evaluate(
    `new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(resolve)))`,
  );
  const desktopState = await devtools.evaluate(`(() => ({
    closed: !document.body.classList.contains("menu-open"),
    collapsed:
      document.querySelector(".menu-button").getAttribute("aria-expanded") ===
      "false",
    overlayHidden: document.querySelector(".mobile-overlay").hidden,
  }))()`);
  if (
    !desktopState.closed ||
    !desktopState.collapsed ||
    !desktopState.overlayHidden
  ) {
    throw new Error(
      `mobile drawer remained open on desktop: ${JSON.stringify(desktopState)}`,
    );
  }
}

export async function verifyResponsiveConsole({ devtools, navigate }) {
  try {
    for (const width of [390, 768, 1024]) {
      await devtools.send("Emulation.setDeviceMetricsOverride", {
        width,
        height: 844,
        deviceScaleFactor: 1,
        mobile: width <= 768,
      });
      for (const path of responsiveConsolePaths) {
        await navigate(path);
        const state = await devtools.evaluate(responsiveStateExpression);
        assertResponsive(path, width, state);
      }
    }
    await verifyDrawer({ devtools, navigate });
  } finally {
    await devtools.send("Emulation.clearDeviceMetricsOverride");
  }
}
