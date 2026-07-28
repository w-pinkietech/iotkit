import { access, mkdtemp, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { createServer } from "node:net";
import { homedir } from "node:os";
import { spawn } from "node:child_process";

import {
  chromiumCandidatePaths,
  chromiumDiagnostics,
  chromiumProfilePrefix,
  removeChromiumProfile,
} from "./profile-cleanup.mjs";
import { verifyResponsiveConsole } from "./responsive-console.mjs";

const origin = process.env.IOTKIT_EDGE_E2E_URL;
const password = process.env.IOTKIT_EDGE_E2E_PASSWORD;
const genericOutputReleasePath =
  process.env.IOTKIT_EDGE_E2E_GENERIC_RELEASE_PATH;
const pinikietOutputReleasePath =
  process.env.IOTKIT_EDGE_E2E_PINIKIET_RELEASE_PATH;
if (!origin || !password) {
  throw new Error("IOTKIT_EDGE_E2E_URL and IOTKIT_EDGE_E2E_PASSWORD are required");
}
if (!genericOutputReleasePath || !pinikietOutputReleasePath) {
  throw new Error(
    "IOTKIT_EDGE_E2E_GENERIC_RELEASE_PATH and IOTKIT_EDGE_E2E_PINIKIET_RELEASE_PATH are required",
  );
}
const viewerPassword = "閲覧担当者の さらに十分に長いパスワード";

const sleep = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds));

async function chromiumExecutables() {
  const candidates = chromiumCandidatePaths(process.env);
  const available = [];
  for (const candidate of candidates) {
    try {
      await access(candidate, constants.X_OK);
      if (!available.includes(candidate)) available.push(candidate);
    } catch {
      // Try the next well-known executable.
    }
  }
  if (available.length === 0) {
    throw new Error("Chromium was not found; set IOTKIT_CHROMIUM");
  }
  return available;
}

async function availablePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  await new Promise((resolve) => server.close(resolve));
  if (!address || typeof address === "string") throw new Error("no debugging port");
  return address.port;
}

async function waitFor(read, description, timeout = 15_000) {
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
      } else if (message.method === "Runtime.exceptionThrown") {
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

  async navigate(path) {
    await this.send("Page.navigate", { url: `${origin}${path}` });
    await waitFor(
      () => this.evaluate("document.readyState === 'complete'"),
      `navigation to ${path}`,
    );
  }
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

const outputDestinationFacts = `(() => {
  const normalize = (value) => value?.replace(/\\s+/g, " ").trim() ?? "";
  return [...document.querySelectorAll(".output-destination-card")]
    .map((card) => ({
      name: normalize(card.querySelector("h2")?.textContent),
      status: normalize(card.querySelector(":scope > header .status-pill")?.textContent),
      summary: Object.fromEntries(
        [...card.querySelectorAll(".output-destination-summary > div")].map((fact) => [
          normalize(fact.querySelector("dt")?.textContent),
          normalize(fact.querySelector("dd")?.textContent),
        ]),
      ),
      technical: [...card.querySelectorAll(".output-technical")]
        .map((details) => normalize(details.textContent))
        .sort(),
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
})()`;

const outputDeliveryDiagnostics = `(() =>
  [...document.querySelectorAll(".output-destination-card")].map((card) => ({
    name: card.querySelector("h2")?.textContent.trim() ?? null,
    status: card.querySelector(":scope > header .status-pill")?.textContent.trim() ?? null,
    rows: [...card.querySelectorAll(".output-rule-row")].map((row) => ({
      ruleName: row.querySelector(":scope > header strong")?.textContent.trim() ?? null,
      status: row.querySelector(":scope > header .status-pill")?.textContent.trim() ?? null,
      payloadSequences: [...row.querySelectorAll(".output-technical pre")]
        .map((payload) => {
          try {
            return JSON.parse(payload.textContent).sequence ?? null;
          } catch {
            return "invalid-json";
          }
        }),
    })),
  }))
)()`;

async function launchBrowser() {
  const failures = [];
  for (const executable of await chromiumExecutables()) {
    const profile = await mkdtemp(chromiumProfilePrefix(process.env, homedir()));
    const debuggingPort = await availablePort();
    let stderr = "";
    const browser = spawn(
      executable,
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
    try {
      const target = await waitFor(async () => {
        const response = await fetch(
          `http://127.0.0.1:${debuggingPort}/json/list`,
        );
        const targets = await response.json();
        return targets.find((candidate) => candidate.type === "page");
      }, `${executable} DevTools page target`, 5_000);
      return { browser, debuggingPort, executable, profile, stderr: () => stderr, target };
    } catch (error) {
      if (browser.exitCode === null && browser.signalCode === null) {
        browser.kill("SIGTERM");
        await new Promise((resolve) => {
          browser.once("close", resolve);
          setTimeout(resolve, 2_000);
        });
      }
      failures.push(
        `${error}\n${chromiumDiagnostics({
          executable,
          exitCode: browser.exitCode,
          signalCode: browser.signalCode,
          stderr,
        })}`,
      );
      await removeChromiumProfile(profile);
    }
  }
  throw new Error(`No browser exposed DevTools:\n${failures.join("\n\n")}`);
}

let socket;
let failure;
let launched;
try {
  launched = await launchBrowser();
  const { target } = launched;
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  const devtools = new DevTools(socket);
  await devtools.send("Page.enable");
  await devtools.send("Runtime.enable");

  await devtools.navigate("/status");
  assert(
    (await devtools.evaluate("location.pathname")) === "/login",
    "anonymous operator was not redirected to login",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("form[action='/login']");
    form.elements.namedItem("login_id").value = "owner";
    form.elements.namedItem("password").value = ${JSON.stringify(password)};
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.pathname === '/status' && document.readyState === 'complete'",
      ),
    "owner login",
  );

  const commissioningEdgeNodeId = "edge-node-commissioning";
  await waitFor(async () => {
    await devtools.navigate("/status");
    return devtools.evaluate(
      `document.querySelector("[data-commissioning-stage='activate-edge-node']")?.textContent.includes("検出した収集ノードを登録してください")`,
    );
  }, "commissioning descriptor discovery");
  const commissioningEdgeNodeHref = await devtools.evaluate(
    `document.querySelector("[data-commissioning-stage='activate-edge-node'] .onboarding-primary")?.getAttribute("href")`,
  );
  assert(
    commissioningEdgeNodeHref?.startsWith("/equipment/edge-nodes/"),
    "commissioning action did not target the exact discovered Edge Node",
  );
  await devtools.navigate(commissioningEdgeNodeHref);
  assert(
    await devtools.evaluate(
      `document.body.textContent.includes(${JSON.stringify(commissioningEdgeNodeId)}) && Boolean(document.querySelector("form[action$='/activation']"))`,
    ),
    "discovered commissioning Edge Node detail was not activatable",
  );
  await devtools.evaluate(
    "document.querySelector(\"form[action$='/activation']\").requestSubmit()",
  );
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && document.readyState === 'complete'",
      ),
    "activation form submission",
  );
  await waitFor(async () => {
    await devtools.navigate(commissioningEdgeNodeHref);
    return devtools.evaluate(
      "document.body.textContent.includes('登録処理中') && !document.querySelector(\"form[action$='/activation']\")",
    );
  }, "single activation entering the in-progress state");

  const commissioningDeviceHref = await waitFor(async () => {
    await devtools.navigate("/status");
    if (
      !(await devtools.evaluate(
        "document.querySelector(\"[data-commissioning-stage='setup-device']\") !== null",
      ))
    ) {
      return undefined;
    }
    return devtools.evaluate(
      "document.querySelector(\"[data-commissioning-stage='setup-device'] .onboarding-primary\")?.getAttribute('href')",
    );
  }, "matching activation result and device setup work");
  await devtools.navigate(commissioningDeviceHref);
  assert(
    await devtools.evaluate(
      "Boolean(document.querySelector(\"form[action^='/console/devices/'][action$='/profile']\")) && document.body.textContent.includes('設定が必要')",
    ),
    "commissioning device was not shown as configuration work",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("form[action^='/console/devices/'][action$='/profile']");
    form.elements.namedItem("display_name").value = "第一工場 蒸気温度計";
    form.elements.namedItem("location").value = "第一工場 ボイラー室";
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && document.body.textContent.includes('第一工場 蒸気温度計')",
      ),
    "commissioning device profile save",
  );
  const commissioningSignalHref = await devtools.evaluate(
    "document.querySelector(\"a[href*='/sensors/']\")?.getAttribute('href')",
  );
  assert(commissioningSignalHref, "commissioning device had no sensor setup link");
  await devtools.navigate(commissioningSignalHref);
  assert(
    await devtools.evaluate(
      "Boolean(document.querySelector('[data-signal-profile]')) && document.body.textContent.includes('設定が必要') && Number.isFinite(Number(document.querySelector('[data-source-value]')?.dataset.sourceValue))",
    ),
    "unconfigured commissioning sensor did not expose its received raw value",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("[data-signal-profile]");
    form.elements.namedItem("display_name").value = "蒸気温度";
    form.elements.namedItem("display_sensor_type").value = "thermocouple";
    form.elements.namedItem("display_value_kind").value = "numeric";
    form.elements.namedItem("display_unit_mode").value = "unit";
    form.elements.namedItem("display_unit").value = "°C";
    form.elements.namedItem("decimal_places").value = "1";
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && document.body.textContent.includes('蒸気温度')",
      ),
    "commissioning sensor basic profile save",
  );
  await devtools.evaluate(`(() => {
    const disclosure = document.querySelector("#rule-create");
    disclosure.open = true;
    const form = disclosure.querySelector(".semantic-form");
    form.elements.namedItem("display_name").value = "現在の蒸気温度";
    form.elements.namedItem("kind").value = "numeric";
    form.requestSubmit();
  })()`);
  try {
    await waitFor(
      () =>
        devtools.evaluate(
          "location.search.includes('saved=1') && document.body.textContent.includes('現在の蒸気温度')",
        ),
      "commissioning numeric semantic rule creation",
    );
  } catch (error) {
    const diagnostic = await devtools.evaluate(
      "JSON.stringify({location: location.href, text: document.body.textContent.slice(0, 1200)})",
    );
    throw new Error(`${error}\nPage state: ${diagnostic}`);
  }
  await devtools.evaluate(`(() => {
    const disclosure = document.querySelector("#rule-create");
    disclosure.open = true;
    const form = disclosure.querySelector(".semantic-form");
    form.elements.namedItem("display_name").value = "蒸気温度通知回数";
    const kind = form.elements.namedItem("kind");
    kind.value = "cumulative_counter";
    kind.dispatchEvent(new Event("change", { bubbles: true }));
    form.elements.namedItem("detector_mode").value = "high_active";
    form.elements.namedItem("rise_threshold").value = "40";
    form.elements.namedItem("fall_threshold").value = "39";
    form.elements.namedItem("rise_debounce_seconds").value = "0";
    form.elements.namedItem("fall_debounce_seconds").value = "0";
    form.elements.namedItem("trigger").value = "on_notification";
    form.requestSubmit();
  })()`);
  try {
    await waitFor(
      () =>
        devtools.evaluate(
          "location.search.includes('saved=1') && document.body.textContent.includes('蒸気温度通知回数')",
        ),
      "commissioning cumulative semantic rule creation",
    );
  } catch (error) {
    const diagnostic = await devtools.evaluate(
      "JSON.stringify({location: location.href, text: document.body.textContent.slice(0, 1200)})",
    );
    throw new Error(`${error}\nPage state: ${diagnostic}`);
  }
  await devtools.navigate("/status");
  assert(
    await devtools.evaluate("document.querySelector('.onboarding') === null"),
    "commissioning panel remained after all required setup",
  );

  assert(
    await devtools.evaluate(
      "Boolean(document.querySelector('.health-banner') && document.querySelector('#signal-table')) && document.body.textContent.includes('乾燥炉入口 温度') && document.body.textContent.includes('29') && document.body.textContent.includes('製造機 青色パトランプ') && document.body.textContent.includes('プレス機 稼働接点') && document.body.textContent.includes('登録済みの収集ノード')",
    ),
    "real stored sensor data was not server-rendered",
  );
  const temperatureHref = await devtools.evaluate(
    `[...document.querySelectorAll("#signal-table tbody tr")]
      .find((row) => row.textContent.includes("乾燥炉入口 温度"))
      ?.querySelector("a")?.getAttribute("href")`,
  );
  const patrolLampHref = await devtools.evaluate(
    `[...document.querySelectorAll("#signal-table tbody tr")]
      .find((row) => row.textContent.includes("製造機 青色パトランプ"))
      ?.querySelector("a")?.getAttribute("href")`,
  );
  const contactHref = await devtools.evaluate(
    `[...document.querySelectorAll("#signal-table tbody tr")]
      .find((row) => row.textContent.includes("プレス機 稼働接点"))
      ?.querySelector("a")?.getAttribute("href")`,
  );
  assert(temperatureHref, "temperature fixture had no equipment link");
  assert(patrolLampHref, "patrol lamp fixture had no equipment link");
  assert(contactHref, "contact fixture had no equipment link");
  await devtools.navigate(patrolLampHref);
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('製造サイクル回数') && document.body.textContent.includes('OFF→ONで +1') && document.body.textContent.includes('ルールを削除')",
    ),
    "configured cumulative rule did not summarize how it counts or expose deletion",
  );

  await devtools.evaluate(`(() => {
    const tab = [...document.querySelectorAll("button[role='tab']")]
      .find((candidate) => candidate.textContent.includes("異常検知"));
    if (!tab) throw new Error("異常検知 tab was not found");
    tab.click();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(`(() => {
        const panel = document.querySelector("[data-setting-panel='alarm']");
        return document.querySelector("button[role='tab'][aria-selected='true']")?.textContent.includes("異常検知") &&
          panel && !panel.hidden &&
          document.querySelector("[data-preview-rule-name]")?.textContent.trim() === "選択中のルールはありません" &&
          !document.querySelector("[data-preview-rule-value]")?.textContent.includes("累積") &&
          [...panel.querySelectorAll("summary")].some((summary) => summary.textContent.trim() === "異常検知を追加");
      })()`),
    "empty patrol lamp alarm tab",
  );
  assert(
    await devtools.evaluate(
      `Boolean([...document.querySelectorAll("[data-setting-panel='alarm'] summary")]
        .find((summary) => summary.textContent.trim() === "異常検知を追加"))`,
    ),
    "patrol lamp alarm tab did not expose its dedicated creation entry",
  );
  await devtools.evaluate(`(() => {
    const draft = [...document.querySelectorAll("details.semantic-rule-create")]
      .find((candidate) => candidate.querySelector("summary")?.textContent.trim() === "異常検知を追加");
    if (!draft) throw new Error("alarm creation disclosure was not found");
    draft.open = true;
    const form = draft.querySelector("form.semantic-form");
    if (!form) throw new Error("alarm creation form was not found");
    const fields = {
      display_name: "照度異常",
      rise_threshold: "900",
      fall_threshold: "850",
    };
    for (const [name, value] of Object.entries(fields)) {
      const field = form.elements.namedItem(name);
      if (!(field instanceof HTMLInputElement)) throw new Error("alarm field " + name + " was not found");
      field.value = value;
      field.dispatchEvent(new Event("input", { bubbles: true }));
      field.dispatchEvent(new Event("change", { bubbles: true }));
    }
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        `document.querySelector("[data-preview-rule-name]")?.textContent.trim() === "照度異常"`,
      ),
    "illuminance alarm draft preview",
  );
  await devtools.evaluate(`(() => {
    const draft = [...document.querySelectorAll("details.semantic-rule-create")]
      .find((candidate) => candidate.querySelector("summary")?.textContent.trim() === "異常検知を追加");
    const form = draft?.querySelector("form.semantic-form");
    if (!form) throw new Error("alarm creation form was not found for submit");
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.pathname.includes('/sensors/') && location.search.includes('saved=1') && location.search.includes('tab=alarm') && document.readyState === 'complete'",
      ),
    "illuminance alarm save and alarm-tab redirect",
  );
  await waitFor(
    () =>
      devtools.evaluate(`(() => {
        const alarmTab = [...document.querySelectorAll("button[role='tab']")]
          .find((candidate) => candidate.textContent.includes("異常検知"));
        const alarmPanel = document.querySelector("[data-setting-panel='alarm']");
        const card = [...(alarmPanel?.querySelectorAll("details.semantic-rule-card") ?? [])]
          .find((candidate) => candidate.querySelector("summary strong")?.textContent.trim() === "照度異常");
        const previewName = document.querySelector("[data-preview-rule-name]")?.textContent.trim();
        const previewValue = document.querySelector("[data-preview-rule-value]")?.textContent.trim() ?? "";
        const accessible = document.querySelector("[data-preview-accessible-summary]")?.textContent ?? "";
        return alarmTab?.getAttribute("aria-selected") === "true" &&
          alarmPanel && !alarmPanel.hidden && card?.open &&
          previewName === "照度異常" && /^(正常|異常)$/.test(previewValue) &&
          accessible.includes("照度異常") && /正常|異常/.test(accessible);
      })()`),
    "saved illuminance alarm selection and preview",
  );
  await devtools.evaluate(`(() => {
    const alarmPanel = document.querySelector("[data-setting-panel='alarm']");
    const card = [...(alarmPanel?.querySelectorAll("details.semantic-rule-card") ?? [])]
      .find((candidate) => candidate.querySelector("summary strong")?.textContent.trim() === "照度異常");
    const form = card?.querySelector("form.semantic-retire-form");
    if (!form) throw new Error("saved illuminance alarm retire form was not found");
    form.removeAttribute("data-confirm-message");
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && !document.body.textContent.includes('照度異常')",
      ),
    "illuminance alarm fixture cleanup",
  );
  await devtools.evaluate(`(() => {
    const card = [...document.querySelectorAll(".semantic-rule-card")]
      .find((candidate) => candidate.textContent.includes("製造サイクル回数"));
    card.open = true;
    const form = card.querySelector(".semantic-retire-form");
    form.removeAttribute("data-confirm-message");
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && !document.body.textContent.includes('製造サイクル回数')",
      ),
    "semantic rule deletion",
  );

  await devtools.navigate(temperatureHref);
  await devtools.evaluate(`(() => {
    const tab = [...document.querySelectorAll("button[role='tab']")]
      .find((candidate) => candidate.textContent.includes("異常検知"));
    if (!tab) throw new Error("temperature alarm tab was not found");
    tab.click();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        `document.querySelector("button[role='tab'][aria-selected='true']")?.textContent.includes("異常検知") && !document.querySelector("[data-setting-panel='alarm']")?.hidden`,
      ),
    "temperature alarm tab",
  );
  await devtools.evaluate(`(() => {
    const panel = document.querySelector("[data-setting-panel='alarm']");
    const card = [...(panel?.querySelectorAll("details.semantic-rule-card") ?? [])]
      .find((candidate) => candidate.querySelector("summary strong")?.textContent.trim() === "高温アラーム");
    if (!card) throw new Error("saved temperature alarm card was not found");
    card.open = true;
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(`(() => {
        const value = document.querySelector("[data-preview-rule-value]")?.textContent.trim() ?? "";
        const name = document.querySelector("[data-preview-rule-name]")?.textContent.trim();
        const accessible = document.querySelector("[data-preview-accessible-summary]")?.textContent ?? "";
        return name === "高温アラーム" && /^(正常|異常)$/.test(value) &&
          accessible.includes("高温アラーム") && /正常|異常/.test(accessible);
      })()`),
    "temperature alarm preview result",
  );

  await devtools.navigate(contactHref);
  await devtools.evaluate(`(() => {
    const tab = [...document.querySelectorAll("button[role='tab']")]
      .find((candidate) => candidate.textContent.includes("計測ルール"));
    if (!tab) throw new Error("contact normal tab was not found");
    tab.click();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        `document.querySelector("button[role='tab'][aria-selected='true']")?.textContent.includes("計測ルール") && !document.querySelector("[data-setting-panel='normal']")?.hidden`,
      ),
    "contact normal tab",
  );
  await devtools.evaluate(`(() => {
    const panel = document.querySelector("[data-setting-panel='normal']");
    const card = [...(panel?.querySelectorAll("details.semantic-rule-card") ?? [])]
      .find((candidate) => candidate.querySelector("summary strong")?.textContent.trim() === "設備稼働");
    if (!card) throw new Error("saved contact boolean card was not found");
    card.open = true;
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(`(() => {
        const value = document.querySelector("[data-preview-rule-value]")?.textContent.trim() ?? "";
        const name = document.querySelector("[data-preview-rule-name]")?.textContent.trim();
        const accessible = document.querySelector("[data-preview-accessible-summary]")?.textContent ?? "";
        return name === "設備稼働" && /^(ON|OFF)$/.test(value) &&
          accessible.includes("設備稼働") && /ON|OFF/.test(accessible);
      })()`),
    "contact boolean preview result",
  );
  await devtools.evaluate(`(() => {
    const panel = document.querySelector("[data-setting-panel='normal']");
    const card = [...(panel?.querySelectorAll("details.semantic-rule-card") ?? [])]
      .find((candidate) => candidate.querySelector("summary strong")?.textContent.trim() === "稼働開始回数");
    if (!card) throw new Error("saved contact cumulative card was not found");
    card.open = true;
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(`(() => {
        const value = document.querySelector("[data-preview-rule-value]")?.textContent.trim() ?? "";
        const name = document.querySelector("[data-preview-rule-name]")?.textContent.trim();
        const accessible = document.querySelector("[data-preview-accessible-summary]")?.textContent ?? "";
        return name === "稼働開始回数" && /^累積 /.test(value) &&
          accessible.includes("稼働開始回数") && accessible.includes("累積");
      })()`),
    "contact cumulative preview result",
  );

  await devtools.navigate("/equipment");
  assert(
    await devtools.evaluate(
      "Boolean(document.querySelector('.equipment-overview') && document.querySelector(\"a[href^='/equipment/edge-nodes/']\")) && document.body.textContent.includes('1台')",
    ),
    "registered Edge Node inventory was not rendered",
  );
  const edgeNodeHref = await devtools.evaluate(
    "document.querySelector(\"a[href^='/equipment/edge-nodes/']\")?.getAttribute('href')",
  );
  await devtools.navigate(edgeNodeHref);
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('乾燥炉入口 熱電対変換器') && Boolean(document.querySelector(\"a[href^='/equipment/devices/']\"))",
    ),
    "device was not reachable from its Edge Node",
  );
  const deviceHref = await devtools.evaluate(
    "document.querySelector(\"a[href^='/equipment/devices/']\")?.getAttribute('href')",
  );
  await devtools.navigate(deviceHref);
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('乾燥炉入口 温度') && Boolean(document.querySelector(\"a[href*='/sensors/']\"))",
    ),
    "sensor was not reachable from its device",
  );

  await devtools.navigate("/status");
  const signalHref = await devtools.evaluate(
    "document.querySelector('#signal-table tbody a')?.getAttribute('href')",
  );
  assert(signalHref, "stored signal had no equipment link");
  await devtools.navigate(signalHref);
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('現在温度') && Boolean(document.querySelector('.semantic-form'))",
    ),
    "stored semantic rule was not rendered",
  );
  try {
    await waitFor(
      () =>
        devtools.evaluate(
          "document.querySelector('[data-preview-current-received]')?.textContent.includes('最終受信')",
        ),
      "fresh fixture receive time",
    );
  } catch (error) {
    const diagnostic = await devtools.evaluate(
      "JSON.stringify({received: document.querySelector('[data-preview-current-received]')?.textContent, message: document.querySelector('[data-preview-message]')?.textContent, range: document.querySelector('[data-preview-range]')?.textContent, count: document.querySelector('[data-preview-count]')?.textContent, location: location.href})",
    );
    throw new Error(`${error}\nPage state: ${diagnostic}`);
  }
  assert(
    await devtools.evaluate(
      "!/\\d{3,}分前/.test(document.querySelector('[data-preview-current-received]')?.textContent ?? '')",
    ),
    "fresh Console fixture was rendered as stale historical data",
  );
  await devtools.evaluate(`(() => {
    const disclosure = document.querySelector("#rule-create");
    disclosure.open = true;
    const kind = disclosure.querySelector("[data-semantic-kind]");
    kind.value = "cumulative_counter";
    kind.dispatchEvent(new Event("change", { bubbles: true }));
  })()`);
  assert(
    await devtools.evaluate(
      "Boolean(!document.querySelector('#rule-create [data-semantic-detector]')?.hidden && !document.querySelector('#rule-create [data-semantic-trigger]')?.hidden && document.querySelector('#rule-create [name=trigger]')?.value === 'on_transition')",
    ),
    "cumulative value change-processing settings were not revealed",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("[data-signal-profile]");
    form.elements.namedItem("display_name").value = "第一ボイラー温度";
    form.elements.namedItem("display_sensor_type").value = "thermocouple";
    form.elements.namedItem("display_value_kind").value = "numeric";
    form.elements.namedItem("display_unit_mode").value = "unit";
    form.elements.namedItem("display_unit").value = "°C";
    form.elements.namedItem("decimal_places").value = "1";
    form.requestSubmit();
  })()`);
  try {
    await waitFor(
      () =>
        devtools.evaluate(
          "location.search.includes('saved=1') && document.body.textContent.includes('第一ボイラー温度')",
        ),
      "signal presentation profile save",
    );
  } catch (error) {
    const diagnostic = await devtools.evaluate(
      "JSON.stringify({location: location.href, text: document.body.textContent.slice(0, 1000)})",
    );
    throw new Error(`${error}\nPage state: ${diagnostic}`);
  }
  await devtools.navigate(signalHref);
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('第一ボイラー温度') && document.querySelector(\"[data-signal-profile] [name='display_unit']\").value === '°C'",
    ),
    "signal presentation profile did not survive reload",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector(".semantic-form");
    form.elements.namedItem("display_name").value = "補正後温度";
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && document.body.textContent.includes('補正後温度')",
      ),
    "semantic rule mutation",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector(".calibration-form");
    form.elements.namedItem("scale").value = "2";
    form.elements.namedItem("offset").value = "1";
    form.requestSubmit();
  })()`);
  await waitFor(
    () => devtools.evaluate("location.search.includes('saved=1')"),
    "calibration mutation",
  );

  await devtools.navigate("/output");
  const inactiveOutputState = await devtools.evaluate(`(() => {
    const summaryCounts = Object.fromEntries(
      [...document.querySelectorAll(".output-health-card")].map((card) => [
        card.querySelector("span")?.textContent.trim(),
        Number(card.querySelector("strong")?.textContent),
      ]),
    );
    const genericAddCard = document.querySelector(
      ".output-add-grid form.output-add-card:has(input[value='iotkit.mqtt-json.v1'])",
    );
    return { summaryCounts, genericAvailable: Boolean(genericAddCard) };
  })()`);
  assert(
    inactiveOutputState.summaryCounts["正常に送信中"] === 0 &&
      inactiveOutputState.summaryCounts["設定が必要"] === 0 &&
      inactiveOutputState.summaryCounts["配送に問題"] === 0 &&
      inactiveOutputState.genericAvailable,
    `output page did not start with zero delivery summaries and Generic MQTT available: ${JSON.stringify(inactiveOutputState)}`,
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector(
      ".output-add-grid form.output-add-card:has(input[value='iotkit.mqtt-json.v1'])",
    );
    form.elements.namedItem("auto_bind_future_rules").checked = true;
    form.requestSubmit();
  })()`);
  try {
    await waitFor(
      () =>
        devtools.evaluate(`(() => {
          const card = [...document.querySelectorAll(".output-destination-card")]
            .find((candidate) => candidate.querySelector("h2")?.textContent === "汎用MQTT JSONで送る");
          return location.pathname === "/output" &&
            location.search.includes("saved=1") &&
            Boolean(card) &&
            !card.querySelector("form.output-binding-form, form.prepared-output-start");
        })()`),
      "active Generic output without actionable bindings",
    );
  } catch (error) {
    const diagnostic = await devtools.evaluate(`JSON.stringify((() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "汎用MQTT JSONで送る");
      return {
        location: location.href,
        cardText: card?.textContent.replace(/\\s+/g, " ").trim() ?? null,
        bindingStatuses: [...(card?.querySelectorAll(".output-rule-row > header .status-pill") ?? [])]
          .map((status) => status.textContent.trim()),
        outputBindingFormCount: card?.querySelectorAll("form.output-binding-form").length ?? 0,
        preparedOutputStartFormCount: card?.querySelectorAll("form.prepared-output-start").length ?? 0,
      };
    })())`);
    throw new Error(`${error}\nPage state: ${diagnostic}`);
  }
  await writeFile(genericOutputReleasePath, "", { flag: "wx" });
  try {
    await waitFor(
      async () => {
        await devtools.navigate("/output");
        return devtools.evaluate(`(() => {
          const card = [...document.querySelectorAll(".output-destination-card")]
            .find((candidate) => candidate.querySelector("h2")?.textContent === "汎用MQTT JSONで送る");
          const destinationStatus = card?.querySelector(":scope > header .status-pill")?.textContent.trim();
          const releasedRule = [...(card?.querySelectorAll(".output-rule-row") ?? [])]
            .find((row) => row.querySelector(":scope > header strong")?.textContent.trim() === "現在の蒸気温度");
          const releasedRuleStatus = releasedRule
            ?.querySelector(":scope > header .status-pill")?.textContent.trim();
          const releasedPayload = [...(releasedRule?.querySelectorAll(".output-technical pre") ?? [])]
            .some((payload) => {
              try {
                return JSON.parse(payload.textContent).sequence === 2;
              } catch {
                return false;
              }
            });
          return destinationStatus === "正常に送信中" &&
            releasedRuleStatus === "正常に送信中" &&
            releasedPayload &&
            card.textContent.includes("送信対象") &&
            card.textContent.includes("最終送信") &&
            card.textContent.includes("配送待ち") &&
            Boolean(card.querySelector(".output-technical"));
        })()`);
      },
      "generic output activation",
    );
  } catch (error) {
    const diagnostic = await devtools.evaluate(outputDeliveryDiagnostics);
    throw new Error(
      `${error}\nOutput delivery state: ${JSON.stringify(diagnostic)}`,
    );
  }
  await devtools.evaluate(`(() => {
    const form = document.querySelector(
      ".output-add-grid form.output-add-card:has(input[value='pinikiet.mqtt.v1'])",
    );
    form.elements.namedItem("auto_bind_future_rules").checked = true;
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && Boolean([...document.querySelectorAll('.output-destination-card')].find((card) => card.querySelector('h2')?.textContent === 'Pinikietへ送る'))",
      ),
    "Pinikiet output preparation",
  );
  assert(
    await devtools.evaluate(`(() => {
      const cards = [...document.querySelectorAll(".output-destination-card")];
      const generic = cards.find((card) => card.querySelector("h2")?.textContent === "汎用MQTT JSONで送る");
      const pinikiet = cards.find((card) => card.querySelector("h2")?.textContent === "Pinikietへ送る");
      const configuration = pinikiet?.querySelector("form.output-binding-form");
      const modes = [...(configuration?.querySelectorAll("select[name='mode'] option") ?? [])]
        .map((option) => [option.value, option.textContent.trim()]);
      return generic?.textContent.includes("正常に送信中") &&
        pinikiet?.textContent.includes("設定が必要") &&
        pinikiet.textContent.includes("製造機 青色パトランプ") &&
        pinikiet.textContent.includes("外部アプリで送信先を登録") &&
        configuration?.querySelector("select[name='mode'][required]") &&
        configuration.querySelector("input[name='_csrf']") &&
        Number(configuration.querySelector("input[name='revision']")?.value) > 0 &&
        modes.some(([value, label]) => value === "onoff" && label === "ON/OFF") &&
        modes.some(([value, label]) => value === "gantt_chart" && label === "稼働状態") &&
        Boolean(pinikiet.querySelector("form.prepared-output-start"));
    })()`),
    "Pinikiet did not expose real mode configuration independently of Generic MQTT delivery",
  );
  let remainingConfigurations = await devtools.evaluate(`(() => {
    const card = [...document.querySelectorAll(".output-destination-card")]
      .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
    return card?.querySelectorAll("form.output-binding-form").length ?? 0;
  })()`);
  assert(
    remainingConfigurations > 0,
    "Pinikiet fixture did not expose a needs-configuration binding",
  );
  while (remainingConfigurations > 0) {
    const previousCount = remainingConfigurations;
    await devtools.evaluate(`(() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
      const form = card?.querySelector("form.output-binding-form");
      if (!form) throw new Error("Pinikiet configuration form was not found");
      form.elements.namedItem("mode").value = "onoff";
      form.requestSubmit();
    })()`);
    await waitFor(
      () =>
        devtools.evaluate(`(() => {
          const card = [...document.querySelectorAll(".output-destination-card")]
            .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
          return location.search.includes("saved=1") &&
            (card?.querySelectorAll("form.output-binding-form").length ?? 0) < ${previousCount};
        })()`),
      "Pinikiet real mode configuration",
    );
    remainingConfigurations = await devtools.evaluate(`(() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
      return card?.querySelectorAll("form.output-binding-form").length ?? 0;
    })()`);
  }
  assert(
    await devtools.evaluate(`(() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
      return card?.querySelector(":scope > header .status-pill")?.textContent.trim() === "外部登録待ち" &&
        !card.querySelector("form.output-binding-form") &&
        Boolean(card.querySelector("form.prepared-output-start"));
    })()`),
    "Pinikiet mode configuration did not advance to external registration wait",
  );
  let remainingRegistrations = await devtools.evaluate(`(() => {
    const card = [...document.querySelectorAll(".output-destination-card")]
      .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
    return card?.querySelectorAll("form.prepared-output-start").length ?? 0;
  })()`);
  assert(remainingRegistrations > 0, "Pinikiet had no external registration work");
  while (remainingRegistrations > 0) {
    const previousCount = remainingRegistrations;
    await devtools.evaluate(`(() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
      const form = card?.querySelector("form.prepared-output-start");
      if (!form) throw new Error("prepared Pinikiet binding was not found");
      form.elements.namedItem("external_registration_complete").checked = true;
      form.requestSubmit();
    })()`);
    await waitFor(
      () =>
        devtools.evaluate(`(() => {
          const card = [...document.querySelectorAll(".output-destination-card")]
            .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
          return location.search.includes("saved=1") &&
            (card?.querySelectorAll("form.prepared-output-start").length ?? 0) < ${previousCount};
        })()`),
      "Pinikiet external registration confirmation",
    );
    remainingRegistrations = await devtools.evaluate(`(() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "Pinikietへ送る");
      return card?.querySelectorAll("form.prepared-output-start").length ?? 0;
    })()`);
  }
  await writeFile(pinikietOutputReleasePath, "", { flag: "wx" });
  try {
    await waitFor(
      async () => {
        await devtools.navigate("/output");
        return devtools.evaluate(`(() => {
          const cards = [...document.querySelectorAll(".output-destination-card")];
          const generic = cards.find((card) => card.querySelector("h2")?.textContent === "汎用MQTT JSONで送る");
          const pinikiet = cards.find((card) => card.querySelector("h2")?.textContent === "Pinikietへ送る");
          const hasHealthyDestination = (card) =>
            card?.querySelector(":scope > header .status-pill")?.textContent.trim() === "正常に送信中";
          const releasedRule = [...(pinikiet?.querySelectorAll(".output-rule-row") ?? [])]
            .find((row) => row.querySelector(":scope > header strong")?.textContent.trim() === "蒸気温度通知回数");
          const releasedRuleStatus = releasedRule
            ?.querySelector(":scope > header .status-pill")?.textContent.trim();
          const hasReleasedPayload = [...(releasedRule?.querySelectorAll(".output-technical pre") ?? [])]
            .some((payload) => {
              try {
                return JSON.parse(payload.textContent).sequence === 3;
              } catch {
                return false;
              }
            });
          return hasHealthyDestination(generic) &&
            hasHealthyDestination(pinikiet) &&
            releasedRuleStatus === "正常に送信中" &&
            hasReleasedPayload &&
            !pinikiet.querySelector("form.output-binding-form") &&
            !pinikiet.querySelector("form.prepared-output-start");
        })()`);
      },
      "Pinikiet output start",
    );
  } catch (error) {
    const diagnostic = await devtools.evaluate(outputDeliveryDiagnostics);
    throw new Error(
      `${error}\nOutput delivery state: ${JSON.stringify(diagnostic)}`,
    );
  }
  const connectedInventory = await devtools.evaluate(`(async () => {
    const [signals, semantics, profiles, routes] = await Promise.all(
      ["/api/v1/signals", "/api/v1/semantic-definitions", "/api/v1/export-profiles", "/api/v1/output-routes"]
        .map(async (path) => (await fetch(path)).json()),
    );
    return [signals.items.length, semantics.items.length, profiles.items.length, routes.items.length];
  })()`);
  assert(
    connectedInventory.every((count) => count > 0),
    `production inventories were empty: ${connectedInventory}`,
  );

  await devtools.navigate("/accounts");
  await devtools.evaluate(`(() => {
    const form = document.querySelector("form[action='/console/accounts']");
    form.elements.namedItem("login_id").value = "viewer";
    form.elements.namedItem("display_name").value = "第一工場 閲覧担当者";
    form.elements.namedItem("role").value = "viewer";
    form.elements.namedItem("temporary_password").value = ${JSON.stringify(password)};
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.pathname === '/accounts' && location.search.includes('saved=1') && document.body.textContent.includes('第一工場 閲覧担当者')",
    ),
    "account creation form",
  );
  await devtools.navigate("/output");
  const ownerOutputFacts = await devtools.evaluate(outputDestinationFacts);
  assert(
    ownerOutputFacts.length === 2 &&
      ownerOutputFacts.some(({ name }) => name === "汎用MQTT JSONで送る") &&
      ownerOutputFacts.some(({ name }) => name === "Pinikietへ送る") &&
      ownerOutputFacts.every(
        ({ status, summary, technical }) =>
          status === "正常に送信中" &&
          Boolean(summary["送信対象"]) &&
          Boolean(summary["最終送信"]) &&
          Boolean(summary["配送待ち"]) &&
          technical.length > 0 &&
          technical.every(Boolean),
      ),
    `owner output facts were incomplete: ${JSON.stringify(ownerOutputFacts)}`,
  );
  await devtools.evaluate(
    "document.querySelector('.logout-form').requestSubmit()",
  );
  await waitFor(
    () => devtools.evaluate("location.pathname === '/login'"),
    "owner logout",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("form[action='/login']");
    form.elements.namedItem("login_id").value = "viewer";
    form.elements.namedItem("password").value = ${JSON.stringify(password)};
    form.requestSubmit();
  })()`);
  await waitFor(
    () => devtools.evaluate("location.pathname === '/password'"),
    "temporary-password login",
  );
  assert(
    await devtools.evaluate(
      "Boolean(document.querySelector(\"form[action='/password']\"))",
    ),
    "temporary account was not forced through password change",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("form[action='/password']");
    form.elements.namedItem("current_password").value = ${JSON.stringify(password)};
    form.elements.namedItem("new_password").value = ${JSON.stringify(viewerPassword)};
    form.requestSubmit();
  })()`);
  await waitFor(
    () => devtools.evaluate("location.pathname === '/login'"),
    "viewer password change",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("form[action='/login']");
    form.elements.namedItem("login_id").value = "viewer";
    form.elements.namedItem("password").value = ${JSON.stringify(viewerPassword)};
    form.requestSubmit();
  })()`);
  await waitFor(
    () => devtools.evaluate("location.pathname === '/status'"),
    "viewer login after password change",
  );
  await devtools.navigate("/output");
  const viewerOutputFacts = await devtools.evaluate(outputDestinationFacts);
  assert(
    JSON.stringify(viewerOutputFacts) === JSON.stringify(ownerOutputFacts),
    `viewer output facts differ from owner facts: ${JSON.stringify({
      owner: ownerOutputFacts,
      viewer: viewerOutputFacts,
    })}`,
  );
  assert(
    await devtools.evaluate(
      `document.body.textContent.includes("閲覧のみ") &&
        !document.querySelector(
          "form.output-add-card, form.output-binding-form, form.prepared-output-start, form.output-stop-form"
        )`,
    ),
    "viewer output page exposed mutation controls",
  );
  await devtools.send("Emulation.setDeviceMetricsOverride", {
    width: 390,
    height: 844,
    deviceScaleFactor: 1,
    mobile: true,
  });
  await devtools.navigate("/output");
  const narrowOutputState = await devtools.evaluate(`(() => {
    [...document.querySelectorAll(".output-technical")].forEach((details) => {
      details.open = true;
    });
    const topic = document.querySelector(".output-technical .copy-row code")?.textContent ?? "";
    const payload = document.querySelector(".output-technical .copy-row pre")?.textContent ?? "";
    const ids = [...document.querySelectorAll(".output-technical dl code")]
      .map((code) => code.textContent ?? "");
    return {
      longTopic: topic.length > 20,
      longPayload: payload.length > 40,
      longId: ids.some((id) => id.length > 20),
    };
  })()`);
  assert(
    narrowOutputState.longTopic &&
      narrowOutputState.longPayload &&
      narrowOutputState.longId,
    `output page lacks long technical facts at 390px: ${JSON.stringify(narrowOutputState)}`,
  );
  await devtools.send("Emulation.clearDeviceMetricsOverride");

  await devtools.evaluate(
    "document.querySelector('.logout-form').requestSubmit()",
  );
  await waitFor(
    () => devtools.evaluate("location.pathname === '/login'"),
    "viewer logout",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("form[action='/login']");
    form.elements.namedItem("login_id").value = "owner";
    form.elements.namedItem("password").value = ${JSON.stringify(password)};
    form.requestSubmit();
  })()`);
  await waitFor(
    () => devtools.evaluate("location.pathname === '/status'"),
    "owner login after viewer journey",
  );
  await verifyResponsiveConsole({
    devtools,
    navigate: (path) => devtools.navigate(path),
  });
  await devtools.navigate("/output");
  assert(
    await devtools.evaluate(`(() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "汎用MQTT JSONで送る");
      return card?.textContent.includes("正常に送信中") &&
        card.textContent.includes("送信対象") &&
        card.textContent.includes("最終送信") &&
        card.textContent.includes("配送待ち") &&
        Boolean(card.querySelector(".output-technical")) &&
        Boolean(card.querySelector(".output-stop-form"));
    })()`),
    "active Generic MQTT destination facts and stop control were not rendered",
  );
  await devtools.evaluate(
    `(() => {
      const card = [...document.querySelectorAll(".output-destination-card")]
        .find((candidate) => candidate.querySelector("h2")?.textContent === "汎用MQTT JSONで送る");
      card.querySelector(".output-stop-form").requestSubmit();
    })()`,
  );
  await waitFor(
    () =>
      devtools.evaluate(
        `location.search.includes("saved=1") &&
          ![...document.querySelectorAll(".output-destination-card")]
            .some((card) => card.querySelector("h2")?.textContent === "汎用MQTT JSONで送る") &&
          Boolean(document.querySelector(
            ".output-add-grid form.output-add-card:has(input[value='iotkit.mqtt-json.v1'])"
          ))`,
      ),
    "output stop mutation",
  );

  for (const [path, expression, description] of [
    [
      "/equipment",
      "Boolean(document.querySelector('.equipment-overview'))",
      "equipment inventory",
    ],
    [
      "/logs",
      "Boolean(document.querySelector('#history-filter') && document.querySelector('.history-chart') && document.querySelector('#log-table'))",
      "history view",
    ],
    [
      "/system",
      "document.body.textContent.includes('保存データの状態') && document.body.textContent.includes('raw受信データ')",
      "storage view",
    ],
    [
      "/accounts",
      "Boolean(document.querySelector(\"form[action='/console/accounts']\"))",
      "account management",
    ],
  ]) {
    await devtools.navigate(path);
    assert(await devtools.evaluate(expression), `${description} was not rendered`);
  }

  const api = await devtools.evaluate(`(async () => {
    const response = await fetch("/api/v1/system/storage");
    return { status: response.status, body: await response.json() };
  })()`);
  assert(
    api.status === 200 &&
      api.body.profile ===
        (process.env.IOTKIT_TEST_STORAGE_PROFILE === "postgres"
          ? "postgres"
          : "embedded"),
    `production storage API failed: ${JSON.stringify(api)}`,
  );
  assert(
    devtools.exceptions.length === 0,
    `browser JavaScript exceptions: ${devtools.exceptions.join("\n")}`,
  );
  console.log("Rust IoTKit Console browser journey passed");
} catch (error) {
  failure = error instanceof Error ? error : new Error(String(error));
} finally {
  if (socket?.readyState === WebSocket.OPEN) socket.close();
  if (
    launched &&
    launched.browser.exitCode === null &&
    launched.browser.signalCode === null
  ) {
    launched.browser.kill("SIGTERM");
    await new Promise((resolve) => {
      launched.browser.once("close", resolve);
      setTimeout(resolve, 2_000);
    });
  }
  if (failure && launched) {
    failure.message += `\n${chromiumDiagnostics({
      executable: launched.executable,
      exitCode: launched.browser.exitCode,
      signalCode: launched.browser.signalCode,
      stderr: launched.stderr(),
    })}`;
  }
  if (launched) await removeChromiumProfile(launched.profile);
}
if (failure) throw failure;
