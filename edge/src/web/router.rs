//! Route inventory kept separate from handlers so parity tests can compare it
//! with the OpenAPI document and the legacy server registration table.

pub const CONSOLE_GET_ROUTES: &[&str] = &[
    "/status",
    "/live",
    "/monitor",
    "/sensors",
    "/sensors/{signal_ref}",
    "/equipment",
    "/equipment/edge-nodes/{edge_node_ref}",
    "/equipment/devices/{device_ref}",
    "/equipment/devices/{device_ref}/sensors/{signal_ref}",
    "/setup",
    "/edge-nodes",
    "/devices",
    "/signals",
    "/logs",
    "/output",
    "/audit",
    "/accounts",
    "/system",
];

pub const CONSOLE_POST_ROUTES: &[&str] = &[
    "/console/devices/{device_ref}/profile",
    "/console/edge-nodes/{edge_node_ref}/activation",
    "/console/signals/{signal_ref}/profile",
    "/console/signals/{signal_ref}/semantic",
    "/console/signals/{signal_ref}/semantic-counter/reset",
    "/console/signals/{signal_ref}/calibration",
    "/console/signals/{signal_ref}/semantic-rules",
    "/console/semantic-rules/{rule_id}",
    "/console/semantic-rules/{rule_id}/retire",
    "/console/semantic-rules/{rule_id}/counter-resets",
    "/console/export-profiles",
    "/console/export-profiles/{profile_id}/stop",
    "/console/output-bindings/{binding_id}",
    "/console/output-bindings/{binding_id}/start",
    "/console/accounts",
    "/console/accounts/{account_ref}",
    "/console/accounts/{account_ref}/disable",
    "/console/accounts/{account_ref}/password",
];

pub const API_GET_ROUTES: &[&str] = &[
    "/api/v1/devices",
    "/api/v1/edge-nodes",
    "/api/v1/signals",
    "/api/v1/system/storage",
    "/api/v1/system/diagnostics",
    "/api/v1/setup/devices",
    "/api/v1/semantic-definitions",
    "/api/v1/signals/{signal_ref}/semantic-configuration",
    "/api/v1/output-adapters",
    "/api/v1/export-profiles",
    "/api/v1/output-bindings/{binding_id}/publication",
    "/api/v1/output-routes",
    "/api/v1/audit-events",
    "/api/v1/accounts",
];

pub const API_POST_ROUTES: &[&str] = &[
    "/api/v1/edge-nodes/{edge_node_ref}/activation",
    "/api/v1/signals/{signal_ref}/semantic-counter/reset",
    "/api/v1/signals/{signal_ref}/semantic-rules",
    "/api/v1/semantic-rules/{rule_id}/counter-resets",
    "/api/v1/export-profiles/preview",
    "/api/v1/export-profiles",
    "/api/v1/export-profiles/{profile_id}/stop",
    "/api/v1/output-bindings/{binding_id}/start",
    "/api/v1/accounts",
    "/api/v1/session/password",
    "/api/v1/mapping-previews",
];

pub const API_PUT_ROUTES: &[&str] = &[
    "/api/v1/devices/{device_ref}/profile",
    "/api/v1/signals/{signal_ref}/profile",
    "/api/v1/signals/{signal_ref}/semantic-definition",
    "/api/v1/signals/{signal_ref}/calibration",
    "/api/v1/semantic-rules/{rule_id}",
    "/api/v1/output-bindings/{binding_id}",
];

pub const API_DELETE_ROUTES: &[&str] = &[
    "/api/v1/signals/{signal_ref}/semantic-definition",
    "/api/v1/semantic-rules/{rule_id}",
];
