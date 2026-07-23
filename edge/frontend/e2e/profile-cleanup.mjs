import { rm } from "node:fs/promises";
import { join } from "node:path";

export async function removeChromiumProfile(profile, remove = rm) {
  await remove(profile, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 100,
  });
}

export function chromiumProfilePrefix(environment, home) {
  const configuredRoot = environment.IOTKIT_EDGE_E2E_TMPDIR;
  return configuredRoot
    ? join(configuredRoot, "iotkit-console-e2e-")
    : join(home, ".iotkit-console-e2e-");
}

export function chromiumDiagnostics({
  executable,
  exitCode,
  signalCode,
  stderr,
}) {
  const processState = [
    `executable=${executable}`,
    `exit=${exitCode ?? "none"}`,
    `signal=${signalCode ?? "none"}`,
  ].join(" ");
  return `Chromium diagnostics:\n${processState}\n${stderr.trim() || "(no stderr captured)"}`;
}
