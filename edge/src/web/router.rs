//! Route inventory kept separate from handlers so parity tests can compare it
//! with the OpenAPI document and the legacy server registration table.

pub const CONSOLE_GET_ROUTES: &[&str] = &[
    "/status",
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
