import { access, mkdtemp } from "node:fs/promises";
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

const origin = process.env.IOTKIT_EDGE_E2E_URL;
const password = process.env.IOTKIT_EDGE_E2E_PASSWORD;
if (!origin || !password) {
  throw new Error("IOTKIT_EDGE_E2E_URL and IOTKIT_EDGE_E2E_PASSWORD are required");
}

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
  assert(
    await devtools.evaluate(
      "Boolean(document.querySelector('.health-banner') && document.querySelector('#signal-table')) && document.body.textContent.includes('contact_state') && document.body.textContent.includes('21.5')",
    ),
    "real stored sensor data was not server-rendered",
  );

  const signalHref = await devtools.evaluate(
    "document.querySelector('#signal-table tbody a')?.getAttribute('href')",
  );
  assert(signalHref, "stored signal had no equipment link");
  await devtools.navigate(signalHref);
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('稼働状態') && Boolean(document.querySelector('.semantic-form'))",
    ),
    "stored semantic rule was not rendered",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector("[data-signal-profile]");
    form.elements.namedItem("display_name").value = "第一ボイラー温度";
    form.elements.namedItem("display_sensor_type").value = "temperature";
    form.elements.namedItem("display_value_kind").value = "numeric";
    form.elements.namedItem("display_unit").value = "°C";
    form.elements.namedItem("decimal_places").value = "1";
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && document.body.textContent.includes('第一ボイラー温度')",
      ),
    "signal presentation profile save",
  );
  await devtools.navigate(signalHref);
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('第一ボイラー温度') && document.querySelector(\"[data-signal-profile] [name='display_unit']\").value === '°C'",
    ),
    "signal presentation profile did not survive reload",
  );
  await devtools.evaluate(`(() => {
    const form = document.querySelector(".semantic-form");
    form.elements.namedItem("display_name").value = "稼働状態（補正済み）";
    form.requestSubmit();
  })()`);
  await waitFor(
    () =>
      devtools.evaluate(
        "location.search.includes('saved=1') && document.body.textContent.includes('稼働状態（補正済み）')",
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
  assert(
    await devtools.evaluate(
      "document.body.textContent.includes('IoTKit MQTT 出力') && document.body.textContent.includes('稼働状態（補正済み）') && Boolean(document.querySelector('.output-stop-form'))",
    ),
    "stored output profile and route were not rendered",
  );
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
  await devtools.evaluate(
    "document.querySelector('.output-stop-form').requestSubmit()",
  );
  await waitFor(
    () => devtools.evaluate("location.search.includes('saved=1')"),
    "output stop mutation",
  );

  for (const [path, expression, description] of [
    [
      "/equipment",
      "Boolean(document.querySelector('.equipment-list'))",
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
  await devtools.evaluate(
    "document.querySelector('.logout-form').requestSubmit()",
  );
  await waitFor(
    () => devtools.evaluate("location.pathname === '/login'"),
    "logout",
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
