import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { parse: parseYaml } = require("yaml");

const required = ["type", "title", "description", "language", "translation_key", "status", "revision"];
const requiredStringPattern = {
  type: /^[A-Za-z0-9][A-Za-z0-9 ._-]*$/,
  language: /^[A-Za-z0-9][A-Za-z0-9 ._-]*$/,
  translation_key: /^[A-Za-z0-9][A-Za-z0-9 ._-]*$/,
  status: /^[A-Za-z0-9][A-Za-z0-9 ._-]*$/,
};

/**
 * Parse markdown YAML frontmatter.
 * @param {string} content
 * @returns {{ metadata: object, body: string } | { error: string }}
 */
export function parseFrontmatterContent(content) {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n/);
  if (!match) return { error: "missing YAML frontmatter" };
  let parsed;
  try {
    parsed = parseYaml(match[1], { uniqueKeys: true });
  } catch (error) {
    return { error: `invalid YAML frontmatter: ${error.message}` };
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return { error: "frontmatter must be a YAML mapping" };
  }
  const metadata = { ...parsed };
  if (typeof metadata.revision === "number" && Number.isInteger(metadata.revision)) {
    metadata.revision = String(metadata.revision);
  }
  return { metadata, body: content.slice(match[0].length) };
}

/** Validate IoTKit required scalar fields; returns list of error messages. */
export function validateRequiredScalars(metadata) {
  const errors = [];
  for (const key of required) {
    const value = metadata[key];
    if (value === undefined || value === null || value === "") {
      errors.push(`missing required field ${key}`);
      continue;
    }
    if (typeof value !== "string") {
      errors.push(`required field ${key} must be a scalar string (or integer revision)`);
      continue;
    }
    if (key === "revision") {
      if (!/^[1-9][0-9]*$/.test(value)) errors.push("revision must be a positive integer");
      continue;
    }
    if (key === "title" || key === "description") continue;
    const pattern = requiredStringPattern[key];
    if (pattern && !pattern.test(value)) {
      errors.push(`required field ${key} has an invalid scalar form`);
    }
  }
  return errors;
}

export { required };
