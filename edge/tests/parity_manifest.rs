use std::{collections::BTreeSet, fs, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Manifest {
    version: u32,
    groups: Vec<Group>,
}

#[derive(Debug, Deserialize)]
struct Group {
    id: String,
    oracle: String,
    rust: String,
}

#[test]
fn parity_manifest_names_every_required_external_surface() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let bytes =
        fs::read(root.join("testdata/edge-parity/v1/manifest.json")).expect("read parity manifest");
    let manifest: Manifest = serde_json::from_slice(&bytes).expect("parse parity manifest");
    assert_eq!(manifest.version, 1);

    let actual = manifest
        .groups
        .iter()
        .map(|group| group.id.as_str())
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "auth-session",
        "backup-restore",
        "cli",
        "console",
        "diagnostics",
        "history-csv",
        "http-api",
        "mqtt-custody",
        "mqtt-output",
        "output-adapters",
        "semantics",
    ]);
    assert_eq!(actual, expected);
    assert!(
        manifest
            .groups
            .iter()
            .all(|group| !group.oracle.is_empty() && !group.rust.is_empty())
    );
}
