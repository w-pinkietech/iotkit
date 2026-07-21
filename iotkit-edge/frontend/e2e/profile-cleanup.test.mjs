import { describe, expect, it } from "vitest";

import { removeChromiumProfile } from "./profile-cleanup.mjs";

describe("removeChromiumProfile", () => {
  it("retries transient non-empty directory cleanup failures", async () => {
    const calls = [];
    await removeChromiumProfile("/tmp/browser-profile", async (path, options) => {
      calls.push({ path, options });
    });

    expect(calls).toEqual([
      {
        path: "/tmp/browser-profile",
        options: {
          recursive: true,
          force: true,
          maxRetries: 5,
          retryDelay: 100,
        },
      },
    ]);
  });
});
