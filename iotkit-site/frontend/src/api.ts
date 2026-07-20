import type { components, operations } from "./generated/site-api";

export type MappingPreviewRequest =
  operations["createMappingPreview"]["requestBody"]["content"]["application/json"];
export type MappingPreviewResponse =
  operations["createMappingPreview"]["responses"][200]["content"]["application/json"];
export type APIError = components["schemas"]["ErrorResponse"];

export type APIResult<T> =
  | { ok: true; value: T }
  | { ok: false; status: number; error: APIError | null };

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object";
}

function isAPIError(value: unknown): value is APIError {
  if (!isRecord(value) || !("error" in value)) return false;
  const error = value.error;
  return (
    isRecord(error) &&
    "code" in error &&
    typeof error.code === "string" &&
    "message" in error &&
    typeof error.message === "string"
  );
}

function isPreviewPoint(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return [
    "received_at",
    "input",
    "input_min",
    "input_max",
    "calibrated",
    "calibrated_min",
    "calibrated_max",
    "sample_count",
  ].every((field) => typeof value[field] === "number");
}

function isPreviewBody(value: unknown): boolean {
  if (!isRecord(value)) return false;
  const kinds = new Set([
    "numeric",
    "boolean",
    "cumulative_counter",
    "alarm",
  ]);
  return (
    typeof value.kind === "string" &&
    kinds.has(value.kind) &&
    typeof value.input_count === "number" &&
    typeof value.plot_count === "number" &&
    (value.points === null ||
      (Array.isArray(value.points) && value.points.every(isPreviewPoint)))
  );
}

function isMappingPreviewResponse(
  value: unknown,
): value is MappingPreviewResponse {
  if (isPreviewBody(value)) return true;
  if (!isRecord(value) || !isRecord(value.calibration)) return false;
  return (
    typeof value.calibration.scale === "number" &&
    typeof value.calibration.offset === "number" &&
    Array.isArray(value.rules) &&
    value.rules.every(
      (rule) =>
        isPreviewBody(rule) &&
        isRecord(rule) &&
        typeof rule.rule_id === "string" &&
        typeof rule.display_name === "string",
    )
  );
}

export async function createMappingPreview(
  request: MappingPreviewRequest,
  csrfToken: string,
  signal: AbortSignal,
): Promise<APIResult<MappingPreviewResponse>> {
  const response = await fetch("/api/v1/mapping-previews", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-CSRF-Token": csrfToken,
    },
    body: JSON.stringify(request),
    signal,
  });
  const payload: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    return {
      ok: false,
      status: response.status,
      error: isAPIError(payload) ? payload : null,
    };
  }
  if (!isMappingPreviewResponse(payload)) {
    return { ok: false, status: response.status, error: null };
  }
  return { ok: true, value: payload };
}
