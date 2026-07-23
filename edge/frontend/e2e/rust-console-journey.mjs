import { access, mkdtemp } from "node:fs/promises";
import { constants } from "node:fs";
import { createServer } from "node:net";
import { homedir } from "node:os";
import { spawn } from "node:child_process";

import {
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

async function chromiumExecutable() {
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
  throw new Error("Chromium was not found; set IOTKIT_CHROMIUM");
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

const profile = await mkdtemp(chromiumProfilePrefix(process.env, homedir()));
const executable = await chromiumExecutable();
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

let socket;
let failure;
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
      "Boolean(document.querySelector('.health-banner') && document.querySelector('#signal-table'))",
    ),
    "overview content was not server-rendered",
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
  if (browser.exitCode === null && browser.signalCode === null) {
    browser.kill("SIGTERM");
    await new Promise((resolve) => {
      browser.once("close", resolve);
      setTimeout(resolve, 2_000);
    });
  }
  if (failure) {
    failure.message += `\n${chromiumDiagnostics({
      executable,
      exitCode: browser.exitCode,
      signalCode: browser.signalCode,
      stderr,
    })}`;
  }
  await removeChromiumProfile(profile);
}
if (failure) throw failure;
