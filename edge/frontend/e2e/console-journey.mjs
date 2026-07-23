import { access, mkdtemp } from "node:fs/promises";
import { constants } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";

import { removeChromiumProfile } from "./profile-cleanup.mjs";

const edgeNodeURL = process.env.IOTKIT_EDGE_E2E_URL;
const password = process.env.IOTKIT_EDGE_E2E_PASSWORD;
if (!edgeNodeURL || !password) {
  throw new Error("IOTKIT_EDGE_E2E_URL and IOTKIT_EDGE_E2E_PASSWORD are required");
}

const sleep = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds));

async function executable() {
  const candidates = [
    process.env.IOTKIT_CHROMIUM,
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      await access(candidate, constants.X_OK);
      return candidate;
    } catch {
      // Try the next well-known executable.
    }
  }
  throw new Error(
    "Chromium was not found; set IOTKIT_CHROMIUM to a Chrome-compatible executable",
  );
}

async function availablePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  await new Promise((resolve) => server.close(resolve));
  if (!address || typeof address === "string") {
    throw new Error("could not reserve a Chromium debugging port");
  }
  return address.port;
}

async function waitFor(read, description, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await read();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await sleep(50);
  }
  throw new Error(
    `Timed out waiting for ${description}${lastError ? `: ${lastError}` : ""}`,
  );
}

class DevTools {
  constructor(socket) {
    this.socket = socket;
    this.sequence = 0;
    this.pending = new Map();
    this.exceptions = [];
    socket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(data);
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(message.error.message));
        else pending.resolve(message.result);
        return;
      }
      if (message.method === "Runtime.exceptionThrown") {
        this.exceptions.push(
          message.params?.exceptionDetails?.exception?.description ??
            message.params?.exceptionDetails?.text ??
            "unknown browser exception",
        );
      }
    });
  }

  send(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (result.exceptionDetails) {
      throw new Error(
        result.exceptionDetails.exception?.description ??
          result.exceptionDetails.text,
      );
    }
    return result.result.value;
  }

  async navigate(url, expectedPath) {
    await this.send("Page.navigate", { url });
    await waitFor(
      () =>
        this.evaluate(
          `document.readyState === "complete" && location.pathname === ${JSON.stringify(expectedPath)}`,
        ),
      `navigation to ${expectedPath}`,
    );
  }

  async waitForExpression(expression, description) {
    return waitFor(() => this.evaluate(expression), description);
  }
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

const anonymousResponse = await fetch(`${edgeNodeURL}/login`);
assert(anonymousResponse.ok, "login page was not available");
assert(
  anonymousResponse.headers.get("cache-control") === "no-store",
  "Console responses must disable caching",
);
assert(
  anonymousResponse.headers.get("x-content-type-options") === "nosniff",
  "Console responses must prevent content sniffing",
);
assert(
  anonymousResponse.headers.get("content-security-policy")?.includes("frame-ancestors 'none'"),
  "Console responses must deny framing through CSP",
);

const setFormValues = (selector, values) => `(() => {
  const form = document.querySelector(${JSON.stringify(selector)});
  if (!form) throw new Error("form not found: " + ${JSON.stringify(selector)});
  const values = ${JSON.stringify(values)};
  for (const [name, value] of Object.entries(values)) {
    const field = form.elements.namedItem(name);
    if (!field) throw new Error("field not found: " + name);
    if (field.type === "checkbox") field.checked = Boolean(value);
    else field.value = String(value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
    field.dispatchEvent(new Event("change", { bubbles: true }));
  }
  form.requestSubmit();
  return true;
})()`;

const click = (selector) => `(() => {
  const element = document.querySelector(${JSON.stringify(selector)});
  if (!element) throw new Error("element not found: " + ${JSON.stringify(selector)});
  element.click();
  return true;
})()`;

const activeNavigation =
  `document.querySelector(".side-nav a[aria-current='page']")?.textContent.trim()`;

const profile = await mkdtemp(join(tmpdir(), "iotkit-console-e2e-"));
const chrome = await executable();
const debuggingPort = await availablePort();
let stderr = "";
const browser = spawn(
  chrome,
  [
    "--headless=new",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--no-first-run",
    "--no-default-browser-check",
    "--no-sandbox",
    "--remote-debugging-address=127.0.0.1",
    `--remote-debugging-port=${debuggingPort}`,
    `--user-data-dir=${profile}`,
    "about:blank",
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
browser.stderr.on("data", (chunk) => {
  if (stderr.length < 16_384) stderr += chunk.toString();
});

let socket;
try {
  const target = await waitFor(async () => {
    const response = await fetch(`http://127.0.0.1:${debuggingPort}/json/list`);
    const targets = await response.json();
    return targets.find((candidate) => candidate.type === "page");
  }, "Chromium page target");
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  const devtools = new DevTools(socket);
  await devtools.send("Page.enable");
  await devtools.send("Runtime.enable");

  await devtools.navigate(`${edgeNodeURL}/status`, "/login");
  assert(
    (await devtools.evaluate("document.querySelector('h1')?.textContent")) ===
      "IoTKitへログイン",
    "anonymous operator did not reach the login page",
  );

  await devtools.evaluate(
    setFormValues("form[action='/login']", {
      login_id: "operator",
      password,
    }),
  );
  await devtools.waitForExpression(
    `location.pathname === "/status" && document.readyState === "complete"`,
    "admin login",
  );
  assert((await devtools.evaluate(activeNavigation)) === "概要", "overview navigation is not active");

  await devtools.navigate(`${edgeNodeURL}/equipment`, "/equipment");
  const inactiveEdgePath = await devtools.evaluate(`(() => {
    const link = [...document.querySelectorAll("a.equipment-row")]
      .find((candidate) => candidate.textContent.includes("assembly-edge-02"));
    return link?.getAttribute("href");
  })()`);
  assert(inactiveEdgePath?.startsWith("/equipment/edge-nodes/"), "inactive EdgeNode detail link was not found");
  await devtools.evaluate(`location.href = ${JSON.stringify(inactiveEdgePath)}`);
  await devtools.waitForExpression(
    `location.pathname === ${JSON.stringify(inactiveEdgePath)} && document.readyState === "complete"`,
    "inactive EdgeNode detail",
  );
  await devtools.evaluate(
    `document.querySelector("form[action*='/activation']").requestSubmit()`,
  );
  await devtools.waitForExpression(
    `location.pathname === ${JSON.stringify(inactiveEdgePath)} && location.search.includes("saved=1") && document.body.textContent.includes("登録処理中")`,
    "EdgeNode activation request",
  );

  await devtools.navigate(`${edgeNodeURL}/sensors`, "/sensors");
  assert((await devtools.evaluate(activeNavigation)) === "センサー一覧", "sensor list navigation is not active");
  const settingsPath = await devtools.evaluate(`(() => {
    const row = [...document.querySelectorAll("#signal-table tbody tr")]
      .find((candidate) => candidate.textContent.includes("factory-edge-01"));
    return row?.querySelector("a")?.getAttribute("href");
  })()`);
  assert(
    settingsPath?.startsWith("/equipment/devices/") && settingsPath.includes("/sensors/"),
    `admin sensor link does not target equipment settings: ${settingsPath}`,
  );
  await devtools.evaluate(`location.href = ${JSON.stringify(settingsPath)}`);
  await devtools.waitForExpression(
    `location.pathname === ${JSON.stringify(settingsPath)} && document.readyState === "complete"`,
    "sensor settings navigation",
  );
  assert((await devtools.evaluate(activeNavigation)) === "機器管理", "settings route did not activate equipment navigation");

  await devtools.evaluate(click("[data-setting-tab='basic']"));
  assert(
    await devtools.evaluate(`!document.querySelector("[data-setting-panel='basic']").hidden`),
    "basic settings tab did not open",
  );
  await devtools.evaluate(`document.querySelector("#sensor-profile").open = true`);
  await devtools.evaluate(
    setFormValues("form[data-signal-profile]", {
      display_name: "乾燥炉入口 温度",
      display_sensor_type: "thermocouple",
      display_value_kind: "numeric",
      display_unit_mode: "dimensionless",
      display_unit: "",
      decimal_places: "1",
    }),
  );
  await devtools.waitForExpression(
    `location.search.includes("saved=1") && document.body.textContent.includes("乾燥炉入口 温度")`,
    "sensor profile save",
  );

  async function addRule(values) {
    await devtools.evaluate(`document.querySelector("#rule-create").open = true`);
    await devtools.evaluate(
      setFormValues("#rule-create form", {
        detector_mode: "high_active",
        rise_threshold: "30",
        fall_threshold: "29",
        rise_debounce_seconds: "0",
        fall_debounce_seconds: "0",
        trigger: "on_transition",
        ...values,
      }),
    );
    await devtools.waitForExpression(
      `location.search.includes("saved=1") && document.body.textContent.includes(${JSON.stringify(values.display_name)})`,
      `rule creation: ${values.display_name}`,
    );
  }

  await addRule({ display_name: "現在温度", kind: "numeric" });
  await addRule({
    display_name: "高温到達回数",
    kind: "cumulative_counter",
  });
  await addRule({ display_name: "高温アラーム", kind: "alarm" });

  await devtools.evaluate(click("[data-setting-tab='alarm']"));
  assert(
    await devtools.evaluate(
      `!document.querySelector("[data-setting-panel='alarm']").hidden && document.querySelector("[data-setting-panel='alarm']").textContent.includes("高温アラーム")`,
    ),
    "alarm rule is not visible in its dedicated panel",
  );
  try {
    await devtools.waitForExpression(
      `document.querySelectorAll("[data-preview-chart] path").length >= 2 && document.querySelector("[data-preview-accessible-summary]").textContent.includes("受信値")`,
      "live signal preview",
    );
  } catch (error) {
    const state = await devtools.evaluate(`({
      url: location.href,
      chart: document.querySelector("[data-preview-chart]")?.outerHTML.slice(0, 1200),
      summary: document.querySelector("[data-preview-accessible-summary]")?.textContent.trim(),
      status: document.querySelector("[data-preview-status]")?.textContent.trim(),
      body: document.body.textContent.replace(/\\s+/g, " ").trim().slice(0, 1200),
    })`);
    error.message += `\nPreview page state: ${JSON.stringify(state)}`;
    if (devtools.exceptions.length) {
      error.message += `\nBrowser exceptions: ${devtools.exceptions.join("\n")}`;
    }
    throw error;
  }

  await devtools.navigate(`${edgeNodeURL}/output`, "/output");
  assert((await devtools.evaluate(activeNavigation)) === "外部出力", "output navigation is not active");
  await devtools.evaluate(
    setFormValues(
      "form.output-add-card:has(input[value='iotkit.mqtt-json.v1'])",
      { auto_bind_future_rules: true },
    ),
  );
  try {
    await devtools.waitForExpression(
      `location.pathname === "/output" && location.search.includes("saved=1") && document.body.textContent.includes("汎用MQTT JSONで送る")`,
      "generic output activation",
    );
  } catch (error) {
    const state = await devtools.evaluate(`({
      url: location.href,
      alert: document.querySelector("[role='alert']")?.textContent.trim(),
      body: document.body.textContent.replace(/\\s+/g, " ").trim().slice(0, 1200),
    })`);
    error.message += `\nOutput page state: ${JSON.stringify(state)}`;
    throw error;
  }
  assert(
    (await devtools.evaluate(
      `document.querySelectorAll(".output-binding-table tbody tr:not(.empty-row)").length`,
    )) === 3,
    "generic output did not bind all three semantic rules",
  );

  await devtools.evaluate(
    setFormValues(
      "form.output-add-card:has(input[value='pinikiet.mqtt.v1'])",
      { auto_bind_future_rules: true },
    ),
  );
  await devtools.waitForExpression(
    `location.pathname === "/output" && location.search.includes("saved=1") && document.body.textContent.includes("Pinikietへ送る")`,
    "Pinikiet output preparation",
  );
  const preparedCount = await devtools.evaluate(`(() => {
    const card = [...document.querySelectorAll(".output-destination-card")]
      .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
    return card?.querySelectorAll("form.prepared-output-start").length;
  })()`);
  assert(
    preparedCount === 1,
    `Pinikiet sensor registration count is ${preparedCount}, want 1`,
  );
  await devtools.evaluate(`(() => {
    const card = [...document.querySelectorAll(".output-destination-card")]
      .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
    const form = card?.querySelector("form.prepared-output-start");
    if (!form) throw new Error("prepared Pinikiet binding was not found");
    form.elements.namedItem("external_registration_complete").checked = true;
    form.requestSubmit();
  })()`);
  await devtools.waitForExpression(
    `location.pathname === "/output" && location.search.includes("saved=1") && (() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
      return card?.querySelectorAll("form.prepared-output-start").length === 0 &&
        card?.textContent.includes("送信対象");
    })()`,
    "Pinikiet output start",
  );

  await devtools.evaluate(`document.querySelector(".logout-form").requestSubmit()`);
  await devtools.waitForExpression(`location.pathname === "/login"`, "logout");
  await devtools.evaluate(
    setFormValues("form[action='/login']", {
      login_id: "viewer",
      password,
    }),
  );
  await devtools.waitForExpression(`location.pathname === "/status"`, "viewer login");
  await devtools.navigate(`${edgeNodeURL}/sensors`, "/sensors");
  const monitoringPath = await devtools.evaluate(`(() => {
    const row = [...document.querySelectorAll("#signal-table tbody tr")]
      .find((candidate) => candidate.textContent.includes("factory-edge-01"));
    return row?.querySelector("a")?.getAttribute("href");
  })()`);
  assert(
    monitoringPath?.startsWith("/sensors/"),
    `viewer sensor link does not target monitoring: ${monitoringPath}`,
  );
  await devtools.evaluate(`location.href = ${JSON.stringify(monitoringPath)}`);
  await devtools.waitForExpression(
    `location.pathname === ${JSON.stringify(monitoringPath)}`,
    "viewer sensor monitoring navigation",
  );
  assert((await devtools.evaluate(activeNavigation)) === "センサー一覧", "viewer detail did not retain sensor navigation");
  assert(
    !(await devtools.evaluate(
      `Boolean(document.querySelector("form[data-signal-profile], form.semantic-form"))`,
    )),
    "viewer can see sensor mutation forms",
  );
  await devtools.navigate(`${edgeNodeURL}/logs`, "/logs");
  assert((await devtools.evaluate(activeNavigation)) === "受信履歴", "history navigation is not active");
  const historyState = await devtools.evaluate(`(() => ({
    filter: Boolean(document.querySelector("#history-filter")),
    chart: document.querySelector("svg.history-chart path")?.getAttribute("d") ?? "",
    rows: document.querySelectorAll("#log-table tbody tr:not(.empty-row)").length,
    body: document.body.textContent.replace(/\\s+/g, " ").trim().slice(0, 1400),
  }))()`);
  assert(
    historyState.filter && historyState.chart && historyState.rows > 0,
    `history filter, chart, and raw table are not available together: ${JSON.stringify(historyState)}`,
  );
  const csvResult = await devtools.evaluate(`(async () => {
    const links = [...document.querySelectorAll("a")];
    const processed = links.find((candidate) => candidate.textContent.includes("加工後CSV"));
    const raw = links.find((candidate) => candidate.textContent.includes("受信した生データCSV"));
    if (!processed || !raw) return { status: 0, type: "", body: "", processed: "", raw: "" };
    const response = await fetch(processed.href);
    return {
      status: response.status,
      type: response.headers.get("content-type") ?? "",
      body: await response.text(),
      processed: new URL(processed.href).pathname,
      raw: new URL(raw.href).pathname,
    };
  })()`);
  assert(
    csvResult.status === 200 && csvResult.type.includes("text/csv") &&
      csvResult.processed === "/api/v1/semantic-history.csv" &&
      csvResult.raw === "/api/v1/history.csv" &&
      csvResult.body.includes("rule_name"),
    `processed and raw history CSV boundary failed: ${JSON.stringify(csvResult)}`,
  );
  await devtools.navigate(`${edgeNodeURL}/system`, "/system");
  const storageState = await devtools.evaluate(`(() => ({
    body: document.body.textContent,
    meter: Boolean(document.querySelector(".storage-meter progress")),
  }))()`);
  const postgresStorage = process.env.IOTKIT_TEST_STORAGE_PROFILE === "postgres";
  assert(
    storageState.body.includes("保存データの状態") &&
      storageState.body.includes("raw受信データ") &&
      storageState.body.includes("確認が必要なこと") &&
      storageState.body.includes("rawの自動削除は無効") &&
      (postgresStorage
        ? storageState.body.includes("PostgreSQL") &&
          storageState.body.includes("hostの空き容量は取得できません") &&
          !storageState.meter
        : storageState.meter),
    "Edge storage facts are not visible in the Console",
  );
  await devtools.navigate(`${edgeNodeURL}/output`, "/output");
  assert(
    await devtools.evaluate(
      `document.body.textContent.includes("閲覧のみ") && !document.querySelector("form.output-add-card")`,
    ),
    "viewer output page does not enforce read-only presentation",
  );

  await devtools.evaluate(`document.querySelector(".logout-form").requestSubmit()`);
  await devtools.waitForExpression(`location.pathname === "/login"`, "viewer logout");
  await devtools.evaluate(
    setFormValues("form[action='/login']", {
      login_id: "owner",
      password,
    }),
  );
  await devtools.waitForExpression(`location.pathname === "/status"`, "system administrator login");
  await devtools.navigate(`${edgeNodeURL}/accounts`, "/accounts");
  assert((await devtools.evaluate(activeNavigation)) === "アカウント", "account navigation is not active");
  assert(
    await devtools.evaluate(
      `Boolean(document.querySelector("form[action='/console/accounts']"))`,
    ),
    "system administrator cannot access account issuance",
  );

  assert(
    devtools.exceptions.length === 0,
    `browser JavaScript exceptions: ${devtools.exceptions.join("\n")}`,
  );
  console.log("IoTKit Console browser journey passed");
} catch (error) {
  if (stderr.trim()) error.message += `\nChromium diagnostics:\n${stderr.trim()}`;
  throw error;
} finally {
  if (socket?.readyState === WebSocket.OPEN) socket.close();
  browser.kill("SIGTERM");
  await new Promise((resolve) => {
    browser.once("exit", resolve);
    setTimeout(resolve, 2_000);
  });
  await removeChromiumProfile(profile);
}
