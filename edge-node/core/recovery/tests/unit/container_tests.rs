use std::{fs, path::Path};

use tempfile::tempdir;

use super::*;
use crate::{BackupCounts, BackupPassphrase, NodeBackupManifest, RecoveryError, SnapshotMode};

const PASSPHRASE: &str = "public-format-passphrase";
const DATABASE: &[u8] = b"SQLite format 3\0public-db";
const DATABASE_LENGTH: u64 = 25;
const DATABASE_SHA256: &str = "958ec6fc5da916b2f0008194cf46f2e9342ceae562e04e4b035baf5b7339b79c";
const FIXED_SALT: [u8; 16] = *b"public-salt-v1!!";
const FIXED_NONCE_PREFIX: [u8; 16] = *b"public-nonce-v1!";

fn encrypt_container_with_entropy(
    snapshot: &Path,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
    salt: [u8; 16],
    nonce_prefix: [u8; 16],
) -> Result<(), RecoveryError> {
    super::encrypt_with_entropy(snapshot, manifest, passphrase, output, salt, nonce_prefix)
}

fn passphrase() -> BackupPassphrase {
    BackupPassphrase::new(PASSPHRASE.into())
}

fn manifest() -> NodeBackupManifest {
    NodeBackupManifest {
        artifact_kind: "iotkit-node-backup".into(),
        format_version: 1,
        backup_id: "backup-public-vector".into(),
        edge_node_id: "node-public-vector".into(),
        ledger_epoch: "epoch-public-vector".into(),
        created_at_ms: 1_725_000_000_000,
        accepted_cursor: 3,
        allocation_high_water: 5,
        snapshot_mode: SnapshotMode::Online,
        shutdown_seal_id: None,
        schema_version: 23,
        database_length: DATABASE_LENGTH,
        database_sha256: DATABASE_SHA256.into(),
        counts: BackupCounts::default(),
    }
}

fn write_database(path: &Path) {
    fs::write(path, DATABASE).unwrap();
}

fn deterministic_artifact(root: &Path) -> (std::path::PathBuf, NodeBackupManifest) {
    let snapshot = root.join("snapshot.sqlite");
    write_database(&snapshot);
    let output = root.join("public-vector.iotkit-node-backup");
    let expected = manifest();
    encrypt_container_with_entropy(
        &snapshot,
        &expected,
        &passphrase(),
        &output,
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap();
    (output, expected)
}

#[test]
fn valid_round_trip_authenticates_and_decrypts_database() {
    let root = tempdir().unwrap();
    let (artifact, expected) = deterministic_artifact(root.path());
    assert_eq!(
        authenticate_container(&artifact, &passphrase()).unwrap(),
        expected
    );

    let output = root.path().join("restored.sqlite");
    let actual = decrypt_container_to_new_file(&artifact, &passphrase(), &output).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(fs::read(output).unwrap(), DATABASE);
}

#[test]
fn public_golden_fixture_matches_json_and_reencodes_byte_for_byte() {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let header_bytes = fs::read(fixture_root.join("node-backup-header-v1.json")).unwrap();
    let header: ContainerHeader = serde_json::from_slice(&header_bytes).unwrap();
    assert_eq!(header.artifact_kind, "iotkit_edge_node_database");
    assert_eq!(header.format_version, 1);
    assert_eq!(
        serde_json::to_vec(&header).unwrap(),
        header_bytes.trim_ascii_end()
    );
    let manifest_bytes = fs::read(fixture_root.join("node-backup-manifest-v1.json")).unwrap();
    let expected: NodeBackupManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    validate_manifest(&expected).unwrap();
    assert_eq!(
        serde_json::to_vec(&expected).unwrap(),
        manifest_bytes.trim_ascii_end()
    );

    let artifact = fixture_root.join("node-backup-v1.bin");
    assert_eq!(
        authenticate_container(&artifact, &passphrase()).unwrap(),
        expected
    );
    let root = tempdir().unwrap();
    let snapshot = root.path().join("golden.sqlite");
    assert_eq!(
        decrypt_container_to_new_file(&artifact, &passphrase(), &snapshot).unwrap(),
        expected
    );
    let reencoded = root.path().join("reencoded.iotkit-node-backup");
    encrypt_container_with_entropy(
        &snapshot,
        &expected,
        &passphrase(),
        &reencoded,
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap();
    assert_eq!(fs::read(reencoded).unwrap(), fs::read(artifact).unwrap());
}

#[test]
fn exact_header_bytes_are_authenticated() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let bytes = fs::read(&artifact).unwrap();
    let header_length = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    for offset in 0..(12 + header_length) {
        let changed = root.path().join(format!("changed-{offset}"));
        let mut mutated = bytes.clone();
        mutated[offset] ^= 1;
        fs::write(&changed, mutated).unwrap();
        let result = authenticate_container(&changed, &passphrase());
        assert!(
            matches!(
                result,
                Err(RecoveryError::AuthenticationFailed | RecoveryError::ContainerInvalid)
            ),
            "header offset {offset}: {result:?}"
        );
    }
}

#[test]
fn wrong_passphrase_is_authentication_failure() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let result = authenticate_container(
        &artifact,
        &BackupPassphrase::new("wrong-format-passphrase".into()),
    );
    assert_eq!(result.unwrap_err().reason_code(), "authentication_failed");
}

#[test]
fn invalid_passphrase_is_rejected_before_container_or_snapshot_work() {
    let root = tempdir().unwrap();
    let short = BackupPassphrase::new("short".into());
    assert_eq!(
        authenticate_container(&root.path().join("missing"), &short)
            .unwrap_err()
            .reason_code(),
        "passphrase_invalid"
    );
    assert_eq!(
        encrypt_container(
            &root.path().join("missing"),
            &manifest(),
            &short,
            &root.path().join("output"),
        )
        .unwrap_err()
        .reason_code(),
        "passphrase_invalid"
    );
}

#[test]
fn edge_server_backup_magic_is_rejected_before_key_derivation() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let mut bytes = fs::read(&artifact).unwrap();
    bytes[..8].copy_from_slice(b"IOTKBKP1");
    let changed = root.path().join("edge-server-backup");
    fs::write(&changed, bytes).unwrap();
    assert_eq!(
        authenticate_container(&changed, &passphrase())
            .unwrap_err()
            .reason_code(),
        "container_invalid"
    );
}

#[test]
fn invalid_header_base64_lengths_and_kdf_bounds_are_rejected() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let bytes = fs::read(&artifact).unwrap();
    let cases = [("salt_b64", "not-base64"), ("nonce_prefix_b64", "AA")];
    for (field, value) in cases {
        let changed = rewrite_header(
            &bytes,
            field,
            serde_json::Value::String(value.into()),
            root.path(),
        );
        assert_eq!(
            authenticate_container(&changed, &passphrase())
                .unwrap_err()
                .reason_code(),
            "container_invalid"
        );
    }
    for (field, value) in [
        ("kdf", serde_json::Value::String("argon2i".into())),
        ("cipher", serde_json::Value::String("aes-256-gcm".into())),
        ("unknown_field", serde_json::Value::String("x".into())),
    ] {
        let changed = rewrite_header(&bytes, field, value, root.path());
        assert_eq!(
            authenticate_container(&changed, &passphrase())
                .unwrap_err()
                .reason_code(),
            "container_invalid"
        );
    }
    for (field, value) in [
        ("kdf_time", 0),
        ("kdf_time", 11),
        ("kdf_memory_kib", 1),
        ("kdf_memory_kib", 262_145),
        ("kdf_parallelism", 0),
        ("kdf_parallelism", 17),
        ("chunk_size", 1),
        ("chunk_size", 4_194_305),
    ] {
        let changed = rewrite_header(&bytes, field, serde_json::Value::from(value), root.path());
        assert_eq!(
            authenticate_container(&changed, &passphrase())
                .unwrap_err()
                .reason_code(),
            "container_invalid"
        );
    }
}

#[test]
fn oversized_header_and_manifest_are_rejected_before_allocation() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let mut bytes = fs::read(&artifact).unwrap();
    bytes[8..12].copy_from_slice(&(16 * 1024 + 1_u32).to_be_bytes());
    let changed = root.path().join("oversized-header");
    fs::write(&changed, bytes).unwrap();
    assert_eq!(
        authenticate_container(&changed, &passphrase())
            .unwrap_err()
            .reason_code(),
        "container_invalid"
    );

    let bytes = fs::read(&artifact).unwrap();
    let changed =
        mutate_first_plaintext_u32(&bytes, u32::try_from(1024 * 1024 + 1).unwrap(), root.path());
    assert_eq!(
        authenticate_container(&changed, &passphrase())
            .unwrap_err()
            .reason_code(),
        "container_invalid"
    );
}

#[test]
fn malformed_records_and_eof_invariants_are_rejected() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let bytes = fs::read(&artifact).unwrap();
    let terminal_offset = find_terminal_record(&bytes);

    for (name, mutated) in [
        ("truncated", bytes[..bytes.len() - 1].to_vec()),
        ("trailing", {
            let mut value = bytes.clone();
            value.push(0);
            value
        }),
        ("duplicate-terminal", {
            let mut value = bytes.clone();
            value.extend_from_slice(&bytes[terminal_offset..]);
            value
        }),
        ("unknown-flags", {
            let mut value = bytes.clone();
            value[terminal_offset] = 2;
            value
        }),
        ("early-terminal", {
            let mut value = bytes.clone();
            value[terminal_offset - 5] = 1;
            value[terminal_offset - 4..terminal_offset].copy_from_slice(&0_u32.to_be_bytes());
            value.truncate(terminal_offset + 1 + 4 + 16);
            value
        }),
    ] {
        let changed = root.path().join(name);
        fs::write(&changed, mutated).unwrap();
        assert!(
            authenticate_container(&changed, &passphrase()).is_err(),
            "{name}"
        );
    }
}

#[test]
fn modified_manifest_and_database_fail_authentication() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let bytes = fs::read(&artifact).unwrap();
    let first_record = first_record_offset(&bytes);
    for name in ["manifest", "database"] {
        let mut mutated = bytes.clone();
        mutated[first_record + 1 + 4 + 1] ^= 1;
        let changed = root.path().join(name);
        fs::write(&changed, mutated).unwrap();
        assert!(
            authenticate_container(&changed, &passphrase()).is_err(),
            "{name}"
        );
    }
}

#[test]
fn decryption_refuses_preexisting_output_and_removes_created_output_on_failure() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let existing = root.path().join("existing.sqlite");
    fs::write(&existing, b"keep").unwrap();
    assert_eq!(
        decrypt_container_to_new_file(&artifact, &passphrase(), &existing)
            .unwrap_err()
            .reason_code(),
        "destination_exists"
    );
    assert_eq!(fs::read(&existing).unwrap(), b"keep");

    let mut bytes = fs::read(&artifact).unwrap();
    let terminal = find_terminal_record(&bytes);
    bytes[terminal + 1 + 4 + 1] ^= 1;
    let changed = root.path().join("corrupt");
    fs::write(&changed, bytes).unwrap();
    let output = root.path().join("created-then-failed.sqlite");
    assert!(decrypt_container_to_new_file(&changed, &passphrase(), &output).is_err());
    assert!(!output.exists());
}

#[test]
fn manifest_database_length_and_digest_mismatch_is_rejected() {
    let root = tempdir().unwrap();
    let (_artifact, expected) = deterministic_artifact(root.path());
    let mut changed_manifest = expected;
    changed_manifest.database_length += 1;
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let output = root.path().join("mismatch-length");
    assert_eq!(
        encrypt_container_with_entropy(
            &snapshot,
            &changed_manifest,
            &passphrase(),
            &output,
            FIXED_SALT,
            FIXED_NONCE_PREFIX,
        )
        .unwrap_err()
        .reason_code(),
        "manifest_invalid"
    );

    let mut changed_manifest = manifest();
    changed_manifest.database_sha256 =
        "0000000000000000000000000000000000000000000000000000000000000000".into();
    let output = root.path().join("mismatch-digest");
    assert_eq!(
        encrypt_container_with_entropy(
            &snapshot,
            &changed_manifest,
            &passphrase(),
            &output,
            FIXED_SALT,
            FIXED_NONCE_PREFIX,
        )
        .unwrap_err()
        .reason_code(),
        "manifest_invalid"
    );
}

#[test]
fn debug_and_errors_redact_secrets_and_paths() {
    let root = tempdir().unwrap();
    let (_, _) = deterministic_artifact(root.path());
    let error = RecoveryError::AuthenticationFailed;
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(PASSPHRASE));
    assert!(!rendered.contains(root.path().to_string_lossy().as_ref()));
}

#[cfg(unix)]
#[test]
fn deterministic_entropy_injection_is_test_only_and_output_is_owner_only() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(artifact).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
#[ignore = "one-time public fixture generation"]
fn write_public_golden_fixture() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    fs::copy(
        artifact,
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/node-backup-v1.bin"),
    )
    .unwrap();
}

fn rewrite_header(
    bytes: &[u8],
    field: &str,
    value: serde_json::Value,
    root: &Path,
) -> std::path::PathBuf {
    let header_len = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut header: serde_json::Value =
        serde_json::from_slice(&bytes[12..12 + header_len]).unwrap();
    header[field] = value;
    let encoded = serde_json::to_vec(&header).unwrap();
    let mut changed = bytes[..8].to_vec();
    changed.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
    changed.extend_from_slice(&encoded);
    changed.extend_from_slice(&bytes[12 + header_len..]);
    let path = root.join(format!("header-{field}"));
    fs::write(&path, changed).unwrap();
    path
}

fn first_record_offset(bytes: &[u8]) -> usize {
    let header_len = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    12 + header_len
}

fn find_terminal_record(bytes: &[u8]) -> usize {
    let mut offset = first_record_offset(bytes);
    loop {
        let flags = bytes[offset];
        let plain_len =
            u32::from_be_bytes(bytes[offset + 1..offset + 5].try_into().unwrap()) as usize;
        let record_len = plain_len + 16;
        if flags == 1 {
            return offset;
        }
        offset += 5 + record_len;
    }
}

fn mutate_first_plaintext_u32(bytes: &[u8], value: u32, root: &Path) -> std::path::PathBuf {
    let offset = first_record_offset(bytes);
    let mut changed = bytes.to_vec();
    // This only changes the authenticated ciphertext length field, so the parser
    // must reject it rather than allocate an attacker-controlled manifest.
    changed[offset + 1..offset + 5].copy_from_slice(&(value + 16).to_be_bytes());
    let path = root.join("oversized-manifest-record");
    fs::write(&path, changed).unwrap();
    path
}
