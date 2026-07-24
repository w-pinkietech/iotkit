import { describe, expect, it } from "vitest";

import * as launch from "./profile-cleanup.mjs";

describe("Chromium launch support", () => {
  it("keeps profiles off quota-limited system tmp by default", () => {
    expect(typeof launch.chromiumProfilePrefix).toBe("function");
    expect(
      launch.chromiumProfilePrefix({}, "/home/runner"),
    ).toBe("/home/runner/.iotkit-console-e2e-");
    expect(
      launch.chromiumProfilePrefix(
        { IOTKIT_EDGE_E2E_TMPDIR: "/work/browser" },
        "/home/runner",
      ),
    ).toBe("/work/browser/iotkit-console-e2e-");
  });

  it("always formats process exit and stderr diagnostics", () => {
    expect(typeof launch.chromiumDiagnostics).toBe("function");
    expect(
      launch.chromiumDiagnostics({
        executable: "/usr/bin/chromium",
        exitCode: null,
        signalCode: "SIGTRAP",
        stderr: "",
      }),
    ).toContain("signal=SIGTRAP");
    expect(
      launch.chromiumDiagnostics({
        executable: "/usr/bin/chromium",
        exitCode: 1,
        signalCode: null,
        stderr: "disk quota exceeded",
      }),
    ).toContain("disk quota exceeded");
  });

  it("prefers real Chrome over distro Chromium shims and keeps fallbacks", () => {
    expect(launch.chromiumCandidatePaths({})).toEqual([
      "/usr/bin/google-chrome",
      "/usr/bin/google-chrome-stable",
      "/usr/bin/chromium",
      "/usr/bin/chromium-browser",
    ]);
    expect(
      launch.chromiumCandidatePaths({ IOTKIT_CHROMIUM: "/opt/chrome" }),
    ).toEqual([
      "/opt/chrome",
      "/usr/bin/google-chrome",
      "/usr/bin/google-chrome-stable",
      "/usr/bin/chromium",
      "/usr/bin/chromium-browser",
    ]);
  });
});
