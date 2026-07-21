import { afterEach, describe, expect, it, vi } from "vitest";
import { createMappingPreview } from "./api";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("mapping preview API client", () => {
  it("sends the session CSRF header and returns a valid response", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          kind: "numeric",
          input_count: 0,
          plot_count: 0,
          points: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = await createMappingPreview(
      { signal_ref: "sig_01" },
      "csrf-token",
      new AbortController().signal,
    );

    expect(result.ok).toBe(true);
    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, request] = fetchMock.mock.calls[0] as [
      string,
      RequestInit,
    ];
    expect(url).toBe("/api/v1/mapping-previews");
    expect(request.headers).toMatchObject({
      "Content-Type": "application/json",
      "X-CSRF-Token": "csrf-token",
    });
  });

  it("rejects a malformed successful response as a protocol failure", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ kind: "numeric" }), { status: 200 }),
      ),
    );

    const result = await createMappingPreview(
      { signal_ref: "sig_01" },
      "csrf-token",
      new AbortController().signal,
    );

    expect(result).toEqual({
      ok: false,
      status: 200,
      error: null,
    });
  });
});
