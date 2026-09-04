#!/usr/bin/env node
// Validates the MQTT Output Adapter v1 fixtures under testdata/observation/v1:
//   - every fixture file matches fixture.schema.json
//   - the topic matches the grammar for its channel
//   - a non-empty payload parses as JSON and matches the kind- or
//     status-form-specific definition
//   - the payload is in canonical form (compact JSON, key order
//     series_id, sequence, uptime_ms, unix_epoch_ms, value) so producer conformance can
//     compare bytes
//   - every file under invalid/ is rejected for the reason it states
// Requires `npm ci --prefix scripts/docs` (ajv).

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { createRequire } from "node:module";

const repoRoot = path.resolve(import.meta.dirname, "..");
const require = createRequire(path.join(repoRoot, "scripts/docs/package.json"));
const Ajv2020 = require("ajv/dist/2020").default;

const fixtureRoot = path.join(repoRoot, "testdata/observation/v1");
const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));

const observationSchema = readJson(path.join(fixtureRoot, "observation.schema.json"));
const statusSchema = readJson(path.join(fixtureRoot, "status.schema.json"));
const fixtureSchema = readJson(path.join(fixtureRoot, "fixture.schema.json"));

// strictRequired is off because fixture.schema.json declares its conditional
// `required` inside if/then, where the properties live on the parent.
const ajv = new Ajv2020({ allErrors: true, strict: true, strictRequired: false });
ajv.addSchema(observationSchema);
ajv.addSchema(statusSchema);
ajv.addSchema(fixtureSchema);

const validateFixture = ajv.getSchema(fixtureSchema.$id);
const validateTopic = {
  observation: ajv.compile({ $ref: `${observationSchema.$id}#/$defs/topic` }),
  status: ajv.compile({ $ref: `${statusSchema.$id}#/$defs/topic` }),
};
const validatePayload = {
  measurement: ajv.compile({ $ref: `${observationSchema.$id}#/$defs/measurement` }),
  "accumulated-count": ajv.compile({ $ref: `${observationSchema.$id}#/$defs/accumulated-count` }),
  state: ajv.compile({ $ref: `${observationSchema.$id}#/$defs/state` }),
  heartbeat: ajv.compile({ $ref: `${statusSchema.$id}#/$defs/heartbeat` }),
  "offline-graceful": ajv.compile({ $ref: `${statusSchema.$id}#/$defs/offline-graceful` }),
  "offline-will": ajv.compile({ $ref: `${statusSchema.$id}#/$defs/offline-will` }),
};

const canonicalKeyOrder = {
  observation: ["series_id", "sequence", "uptime_ms", "unix_epoch_ms", "value"],
  status: ["uptime_ms", "unix_epoch_ms", "value", "faults"],
};

function canonical(channel, parsed) {
  const ordered = {};
  for (const key of canonicalKeyOrder[channel]) ordered[key] = parsed[key];
  return JSON.stringify(ordered);
}

// Returns the list of problems with one publication; empty means valid.
function problems(fixture) {
  const found = [];
  const formatErrors = (errors) => (errors ?? []).map((e) => `${e.instancePath || "/"} ${e.message}`).join("; ");
  const channel = fixture.channel;
  if (!validateTopic[channel]?.(fixture.topic)) {
    found.push(`topic does not match the ${channel} grammar: ${fixture.topic}`);
  }
  if (channel === "observation" && fixture.kind) {
    const kindKey = fixture.topic.split("/").at(-1);
    if (kindKey !== fixture.kind) found.push(`topic kind-key ${kindKey} differs from fixture kind ${fixture.kind}`);
  }
  if (fixture.payload === "") {
    if (channel !== "observation") found.push("only observation topics carry a zero-length deletion payload");
    return found;
  }
  let parsed;
  try {
    parsed = JSON.parse(fixture.payload);
  } catch (error) {
    found.push(`payload is not JSON: ${error.message}`);
    return found;
  }
  const form = channel === "observation" ? fixture.kind : fixture.status_form;
  const validate = validatePayload[form];
  if (!validate) {
    found.push(`no payload definition for ${form}`);
    return found;
  }
  if (!validate(parsed)) found.push(`payload violates ${form}: ${formatErrors(validate.errors)}`);
  if (found.length === 0 && canonical(channel, parsed) !== fixture.payload) {
    found.push(`payload is not canonical; expected ${canonical(channel, parsed)}`);
  }
  return found;
}

let failures = 0;
const fail = (file, message) => {
  failures += 1;
  console.error(`${path.relative(repoRoot, file)}: ${message}`);
};

const validFiles = fs
  .readdirSync(fixtureRoot)
  .filter((name) => name.endsWith(".json") && !name.endsWith(".schema.json"))
  .map((name) => path.join(fixtureRoot, name));
for (const file of validFiles) {
  const fixture = readJson(file);
  if (!validateFixture(fixture)) {
    fail(file, `fixture shape: ${(validateFixture.errors ?? []).map((e) => `${e.instancePath || "/"} ${e.message}`).join("; ")}`);
    continue;
  }
  for (const problem of problems(fixture)) fail(file, problem);
}

const invalidDir = path.join(fixtureRoot, "invalid");
const invalidFiles = fs
  .readdirSync(invalidDir)
  .filter((name) => name.endsWith(".json"))
  .map((name) => path.join(invalidDir, name));
for (const file of invalidFiles) {
  const fixture = readJson(file);
  if (typeof fixture.reason !== "string" || fixture.reason.length === 0) {
    fail(file, "invalid fixture must state its reason");
    continue;
  }
  if (problems({ qos: 1, retain: true, ...fixture }).length === 0) {
    fail(file, `expected rejection (${fixture.reason}) but the fixture validated`);
  }
}

if (failures > 0) {
  console.error(`observation fixtures: ${failures} problem(s)`);
  process.exit(1);
}
console.log(`observation fixtures: OK (${validFiles.length} publications, ${invalidFiles.length} rejected as expected)`);
