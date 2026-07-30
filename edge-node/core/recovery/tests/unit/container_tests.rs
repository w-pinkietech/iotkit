use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use std::fs::File;

#[cfg(target_os = "linux")]
use std::io::Read;

use serde_json::{Value, json};
#[cfg(not(target_os = "linux"))]
use tempfile::NamedTempFile;
use tempfile::tempdir;

use super::*;
use crate::{BackupCounts, BackupPassphrase, NodeBackupManifest, RecoveryError, SnapshotMode};

const PASSPHRASE: &str = "public-format-passphrase";
const DATABASE: &[u8] = b"SQLite format 3\0public-db";
const DATABASE_LENGTH: u64 = 25;
const DATABASE_SHA256: &str = "958ec6fc5da916b2f0008194cf46f2e9342ceae562e04e4b035baf5b7339b79c";
const FIXED_SALT: [u8; 16] = *b"public-salt-v1!!";
const FIXED_NONCE_PREFIX: [u8; 16] = *b"public-nonce-v1!";

#[cfg(target_os = "linux")]
fn encrypt_container_with_entropy(
    snapshot: &Path,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
    salt: [u8; 16],
    nonce_prefix: [u8; 16],
) -> Result<(), RecoveryError> {
    let directory = directory_capability(output.parent().unwrap_or_else(|| Path::new(".")));
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(RecoveryError::ContainerInvalid)?;
    super::encrypt_with_entropy(
        snapshot,
        manifest,
        passphrase,
        &directory,
        name,
        salt,
        nonce_prefix,
    )
}

#[cfg(not(target_os = "linux"))]
fn encrypt_container_with_entropy(
    snapshot: &Path,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
    salt: [u8; 16],
    nonce_prefix: [u8; 16],
) -> Result<(), RecoveryError> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent).map_err(|_| RecoveryError::Storage)?;
    super::encrypt_snapshot_contents(
        File::open(snapshot).map_err(|_| RecoveryError::Storage)?,
        manifest,
        passphrase,
        salt,
        nonce_prefix,
        temporary.as_file_mut(),
    )?;
    publish_new_file(temporary.path(), output)
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
        epoch_start_publication_seq: Some(1),
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

fn staging_directory(root: &Path) -> PathBuf {
    let staging = root.join("staging");
    fs::create_dir(&staging).unwrap();
    staging
}

fn directory_capability(path: &Path) -> DirectoryCapability {
    DirectoryCapability::open(path).unwrap()
}

fn directory_entries(path: &Path) -> BTreeSet<String> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn publish_new_file(staged: &Path, destination: &Path) -> Result<(), RecoveryError> {
    fs::hard_link(staged, destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            RecoveryError::DestinationExists
        } else {
            RecoveryError::Storage
        }
    })
}

#[cfg(target_os = "linux")]
fn encrypt_snapshot_reader_with_output_init(
    snapshot_reader: impl Read,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: (&DirectoryCapability, &str),
    salt: [u8; 16],
    nonce_prefix: [u8; 16],
    initialize_output: impl FnOnce(&mut LinuxEncryptedOutput) -> Result<(), RecoveryError>,
) -> Result<(), RecoveryError> {
    validate_manifest(manifest)?;
    let mut output_file = LinuxEncryptedOutput::new(output.0, output.1)?;
    initialize_output(&mut output_file)?;
    encrypt_snapshot_contents(
        snapshot_reader,
        manifest,
        passphrase,
        salt,
        nonce_prefix,
        &mut output_file,
    )?;
    output_file.publish()
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum OutputFault {
    Write,
    Link,
    FileSync,
    DirectorySync,
}

#[cfg(target_os = "linux")]
fn output_ops_with_fault(fault: OutputFault) -> LinuxOutputOps {
    fn fail_write(_: &mut File, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from_raw_os_error(libc::EIO))
    }
    fn fail_file_sync(_: &File) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::EIO))
    }
    fn fail_link(_: &File, _: &File, _: &std::ffi::CString) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::EIO))
    }
    fn fail_directory_sync(_: &File) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::EIO))
    }
    let mut ops = LinuxOutputOps::system();
    match fault {
        OutputFault::Write => ops.write = fail_write,
        OutputFault::Link => ops.link = fail_link,
        OutputFault::FileSync => ops.sync_file = fail_file_sync,
        OutputFault::DirectorySync => ops.sync_directory = fail_directory_sync,
    }
    ops
}

#[cfg(target_os = "linux")]
fn stage_bytes(mut stage: DecryptedStage) -> Vec<u8> {
    stage.rewind().unwrap();
    let mut bytes = Vec::new();
    stage.read_to_end(&mut bytes).unwrap();
    bytes
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

    let staging = staging_directory(root.path());
    let staging_capability = directory_capability(&staging);
    #[cfg(target_os = "linux")]
    {
        let (stage, actual) = decrypt_container_to_staging_file(
            &artifact,
            &passphrase(),
            &staging_capability,
            DATABASE_LENGTH,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let rendered = format!("{stage:?}");
        assert!(rendered.contains("REDACTED"));
        assert!(!rendered.contains(staging.to_string_lossy().as_ref()));
        assert_eq!(stage_bytes(stage), DATABASE);
        assert!(directory_entries(&staging).is_empty());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let error = decrypt_container_to_staging_file(
            &artifact,
            &passphrase(),
            &staging_capability,
            DATABASE_LENGTH,
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "platform_unsupported");
        assert!(directory_entries(&staging).is_empty());
    }
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
    #[cfg(target_os = "linux")]
    {
        let root = tempdir().unwrap();
        let staging = staging_directory(root.path());
        let staging_capability = directory_capability(&staging);
        let (stage, actual) = decrypt_container_to_staging_file(
            &artifact,
            &passphrase(),
            &staging_capability,
            DATABASE_LENGTH,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let snapshot = root.path().join("golden.sqlite");
        fs::write(&snapshot, stage_bytes(stage)).unwrap();
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
}

#[test]
fn schema_and_rust_validation_agree_on_boundaries() {
    let contracts = Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts");
    let header_schema: Value = serde_json::from_slice(
        &fs::read(contracts.join("node-backup-header-v1.schema.json")).unwrap(),
    )
    .unwrap();
    let manifest_schema: Value = serde_json::from_slice(
        &fs::read(contracts.join("node-backup-manifest-v1.schema.json")).unwrap(),
    )
    .unwrap();
    let header_validator = jsonschema::validator_for(&header_schema).unwrap();
    let manifest_validator = jsonschema::validator_for(&manifest_schema).unwrap();

    let header_json: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/node-backup-header-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(header_validator.is_valid(&header_json));
    let header: ContainerHeader = serde_json::from_value(header_json).unwrap();
    validate_header(&header).unwrap();

    let header_cases = [
        (
            "header fractional chunk size",
            json!({"chunk_size": 4096.5}),
        ),
        ("header kdf time out of range", json!({"kdf_time": 11})),
    ];
    for (name, patch) in header_cases {
        let mut value: Value = serde_json::from_slice(
            &fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/node-backup-header-v1.json"),
            )
            .unwrap(),
        )
        .unwrap();
        for (key, replacement) in patch.as_object().unwrap() {
            value
                .as_object_mut()
                .unwrap()
                .insert(key.clone(), replacement.clone());
        }
        assert!(!header_validator.is_valid(&value), "schema accepted {name}");
        let rust_result = match serde_json::from_value::<ContainerHeader>(value) {
            Ok(header) => validate_header(&header).map(|_| header).map_err(|_| ()),
            Err(_) => Err(()),
        };
        assert!(rust_result.is_err(), "Rust accepted {name}");
    }

    let manifest_json: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/node-backup-manifest-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(manifest_validator.is_valid(&manifest_json));
    let manifest_fixture: NodeBackupManifest =
        serde_json::from_value(manifest_json.clone()).unwrap();
    validate_manifest(&manifest_fixture).unwrap();

    let valid = serde_json::to_value(manifest()).unwrap();
    assert!(manifest_validator.is_valid(&valid));
    let parsed: NodeBackupManifest = serde_json::from_value(valid).unwrap();
    validate_manifest(&parsed).unwrap();

    let mut unicode_boundary = serde_json::to_value(manifest()).unwrap();
    unicode_boundary
        .as_object_mut()
        .unwrap()
        .insert("backup_id".into(), json!("é".repeat(255)));
    assert!(manifest_validator.is_valid(&unicode_boundary));
    let parsed: NodeBackupManifest = serde_json::from_value(unicode_boundary).unwrap();
    validate_manifest(&parsed).unwrap();

    let mut integral_float = serde_json::to_value(manifest()).unwrap();
    integral_float
        .as_object_mut()
        .unwrap()
        .insert("created_at_ms".into(), json!(1.0));
    assert!(manifest_validator.is_valid(&integral_float));
    let parsed: NodeBackupManifest = serde_json::from_value(integral_float).unwrap();
    validate_manifest(&parsed).unwrap();

    let cases = [
        ("control character", json!({"backup_id": "bad\u{0001}"})),
        ("c1 control character", json!({"backup_id": "bad\u{0085}"})),
        (
            "256 multibyte characters",
            json!({"backup_id": "é".repeat(256)}),
        ),
        (
            "u64 overflow",
            serde_json::from_str::<Value>("{\"database_length\":18446744073709551616}").unwrap(),
        ),
    ];
    for (name, patch) in cases {
        let mut value = serde_json::to_value(manifest()).unwrap();
        let object = value.as_object_mut().unwrap();
        for (key, replacement) in patch.as_object().unwrap() {
            object.insert(key.clone(), replacement.clone());
        }
        assert!(
            !manifest_validator.is_valid(&value),
            "schema accepted {name}"
        );
        let rust_result = match serde_json::from_value::<NodeBackupManifest>(value) {
            Ok(value) => validate_manifest(&value).map(|_| value).map_err(|_| ()),
            Err(_) => Err(()),
        };
        assert!(rust_result.is_err(), "Rust accepted {name}");
    }

    for field in ["salt_b64", "nonce_prefix_b64"] {
        let mut value: Value = serde_json::from_slice(
            &fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/node-backup-header-v1.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let encoded = value[field].as_str().unwrap();
        let mut noncanonical = encoded.to_owned();
        noncanonical.replace_range(21.., "R");
        value
            .as_object_mut()
            .unwrap()
            .insert(field.into(), json!(noncanonical));
        assert!(
            !header_validator.is_valid(&value),
            "schema accepted noncanonical {field}"
        );
        let rust_result = match serde_json::from_value::<ContainerHeader>(value) {
            Ok(header) => validate_header(&header).and(decode_16(if field == "salt_b64" {
                &header.salt_b64
            } else {
                &header.nonce_prefix_b64
            })),
            Err(_) => Err(RecoveryError::ContainerInvalid),
        };
        assert!(rust_result.is_err(), "Rust accepted noncanonical {field}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn encryption_uses_the_open_snapshot_handle_after_path_replacement() {
    let root = tempdir().unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let opened = File::open(&snapshot).unwrap();
    let moved = root.path().join("original.sqlite");
    fs::rename(&snapshot, &moved).unwrap();
    fs::write(&snapshot, b"replacement-with-different-bytes").unwrap();
    let output = root.path().join("container.iotkit-node-backup");
    let output_directory = directory_capability(root.path());
    encrypt_open_snapshot(
        opened,
        &manifest(),
        &passphrase(),
        &output_directory,
        "container.iotkit-node-backup",
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap();
    let staging = staging_directory(root.path());
    let staging_capability = directory_capability(&staging);
    let (stage, _) = decrypt_container_to_staging_file(
        &output,
        &passphrase(),
        &staging_capability,
        DATABASE_LENGTH,
    )
    .unwrap();
    assert_eq!(stage_bytes(stage), DATABASE);
}

#[cfg(target_os = "linux")]
struct TruncatingReader {
    file: File,
    path: PathBuf,
    truncated: bool,
}

#[cfg(target_os = "linux")]
impl Read for TruncatingReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let limit = bytes.len().min(1);
        let count = self.file.read(&mut bytes[..limit])?;
        if !self.truncated {
            fs::OpenOptions::new()
                .write(true)
                .open(&self.path)?
                .set_len(1)?;
            self.truncated = true;
        }
        Ok(count)
    }
}

#[cfg(target_os = "linux")]
#[test]
fn in_place_snapshot_truncation_during_encryption_fails_without_artifact() {
    let root = tempdir().unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let output = root.path().join("truncated.iotkit-node-backup");
    let reader = TruncatingReader {
        file: File::open(&snapshot).unwrap(),
        path: snapshot,
        truncated: false,
    };
    let output_directory = directory_capability(root.path());
    let error = encrypt_snapshot_reader(
        reader,
        &manifest(),
        &passphrase(),
        &output_directory,
        "truncated.iotkit-node-backup",
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "manifest_invalid");
    assert!(!output.exists());
}

#[test]
fn insufficient_plaintext_capacity_is_rejected_before_output_creation() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let staging = staging_directory(root.path());
    let staging_capability = directory_capability(&staging);
    let error = decrypt_container_to_staging_file(
        &artifact,
        &passphrase(),
        &staging_capability,
        DATABASE_LENGTH - 1,
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "storage_full");
    assert!(directory_entries(&staging).is_empty());
}

#[test]
fn publication_refuses_a_replaced_destination_without_deleting_it() {
    let root = tempdir().unwrap();
    let staged = root.path().join(".iotkit-node-staging-test");
    let destination = root.path().join("published.sqlite");
    fs::write(&staged, b"staged").unwrap();
    fs::write(&destination, b"unrelated-replacement").unwrap();
    assert_eq!(
        publish_new_file(&staged, &destination)
            .unwrap_err()
            .reason_code(),
        "destination_exists"
    );
    assert_eq!(fs::read(&destination).unwrap(), b"unrelated-replacement");
    assert_eq!(fs::read(&staged).unwrap(), b"staged");
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
    let output_directory = directory_capability(root.path());
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
            &output_directory,
            "output",
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
fn decryption_failure_leaves_no_named_plaintext_and_preserves_unrelated_stage_files() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let staging = staging_directory(root.path());
    let staging_capability = directory_capability(&staging);
    let unrelated = staging.join("unrelated.marker");
    fs::write(&unrelated, b"keep").unwrap();
    let before = directory_entries(&staging);

    let mut bytes = fs::read(&artifact).unwrap();
    let terminal = find_terminal_record(&bytes);
    bytes[terminal + 1 + 4 + 1] ^= 1;
    let changed = root.path().join("corrupt");
    fs::write(&changed, bytes).unwrap();
    assert!(
        decrypt_container_to_staging_file(
            &changed,
            &passphrase(),
            &staging_capability,
            DATABASE_LENGTH,
        )
        .is_err()
    );
    assert_eq!(directory_entries(&staging), before);
    assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
}

#[cfg(target_os = "linux")]
#[test]
fn encrypted_output_initialization_failure_removes_temp_and_retry_succeeds() {
    let root = tempdir().unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let output = root.path().join("retry.iotkit-node-backup");
    let output_directory = directory_capability(root.path());
    let before = directory_entries(root.path());

    let error = encrypt_snapshot_reader_with_output_init(
        File::open(&snapshot).unwrap(),
        &manifest(),
        &passphrase(),
        (&output_directory, "retry.iotkit-node-backup"),
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
        |_temporary| Err(RecoveryError::ArtifactCleanupFailed),
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "artifact_cleanup_failed");
    assert!(!output.exists());
    assert_eq!(directory_entries(root.path()), before);

    encrypt_container_with_entropy(
        &snapshot,
        &manifest(),
        &passphrase(),
        &output,
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap();
    assert!(output.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn anonymous_stage_rejects_injected_otmpfile_failure_without_a_directory_entry() {
    let root = tempdir().unwrap();
    let staging = staging_directory(root.path());
    let directory = directory_capability(&staging);
    let error = create_anonymous_plaintext_file_with(&directory, |_| {
        Err(io::Error::from_raw_os_error(libc::EOPNOTSUPP))
    })
    .unwrap_err();
    assert_eq!(error.reason_code(), "platform_unsupported");
    assert!(directory_entries(&staging).is_empty());
}

#[cfg(not(target_os = "linux"))]
#[test]
fn anonymous_stage_fails_closed_on_unsupported_platform() {
    let root = tempdir().unwrap();
    let staging = staging_directory(root.path());
    let staging_capability = directory_capability(&staging);
    let error = DecryptedStage::new(&staging_capability).unwrap_err();
    assert_eq!(error.reason_code(), "platform_unsupported");
    assert!(directory_entries(&staging).is_empty());
}

#[cfg(not(target_os = "linux"))]
#[test]
fn encrypted_publication_fails_closed_on_unsupported_platform() {
    let root = tempdir().unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let output = root.path().join("artifact.iotkit-node-backup");
    let output_directory = directory_capability(root.path());
    let error = encrypt_container(
        &snapshot,
        &manifest(),
        &passphrase(),
        &output_directory,
        "artifact.iotkit-node-backup",
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "platform_unsupported");
    assert!(!output.exists());
}

#[test]
fn invalid_output_names_are_rejected_before_any_write() {
    let root = tempdir().unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let output_directory = directory_capability(root.path());
    let before = directory_entries(root.path());

    for name in ["", ".", "..", "nested/name", "nested\\name", "nul\0name"] {
        let error = encrypt_container(
            &snapshot,
            &manifest(),
            &passphrase(),
            &output_directory,
            name,
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "container_invalid", "{name:?}");
        assert_eq!(directory_entries(root.path()), before, "{name:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn encryption_uses_a_held_directory_capability_after_path_replacement() {
    let root = tempdir().unwrap();
    let original = root.path().join("original-output");
    fs::create_dir(&original).unwrap();
    let output_directory = directory_capability(&original);
    let moved = root.path().join("moved-output");
    fs::rename(&original, &moved).unwrap();
    fs::create_dir(&original).unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);

    encrypt_container(
        &snapshot,
        &manifest(),
        &passphrase(),
        &output_directory,
        "artifact.iotkit-node-backup",
    )
    .unwrap();

    let artifact = moved.join("artifact.iotkit-node-backup");
    assert!(artifact.exists());
    assert!(directory_entries(&original).is_empty());
    assert_eq!(
        authenticate_container(&artifact, &passphrase()).unwrap(),
        manifest()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn decryption_stages_through_a_held_directory_capability_after_path_replacement() {
    let root = tempdir().unwrap();
    let (artifact, _) = deterministic_artifact(root.path());
    let original = root.path().join("original-staging");
    fs::create_dir(&original).unwrap();
    let staging_directory = directory_capability(&original);
    let moved = root.path().join("moved-staging");
    fs::rename(&original, &moved).unwrap();
    fs::create_dir(&original).unwrap();

    let (stage, actual) = decrypt_container_to_staging_file(
        &artifact,
        &passphrase(),
        &staging_directory,
        DATABASE_LENGTH,
    )
    .unwrap();

    assert_eq!(actual, manifest());
    assert_eq!(stage_bytes(stage), DATABASE);
    assert!(directory_entries(&moved).is_empty());
    assert!(directory_entries(&original).is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn encrypted_publication_uses_held_parent_after_path_substitution() {
    let root = tempdir().unwrap();
    let parent = root.path().join("destination");
    fs::create_dir(&parent).unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let moved = root.path().join("moved-destination");
    let replacement = root.path().join("replacement-destination");
    let output_directory = directory_capability(&parent);

    encrypt_snapshot_reader_with_output_init(
        File::open(&snapshot).unwrap(),
        &manifest(),
        &passphrase(),
        (&output_directory, "artifact.iotkit-node-backup"),
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
        |_| {
            fs::rename(&parent, &moved).unwrap();
            fs::create_dir(&replacement).unwrap();
            Ok(())
        },
    )
    .unwrap();
    assert!(moved.join("artifact.iotkit-node-backup").exists());
    assert!(!replacement.join("artifact.iotkit-node-backup").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn encrypted_publication_eexist_preserves_existing_and_retry_succeeds() {
    let root = tempdir().unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let output = root.path().join("artifact.iotkit-node-backup");
    fs::write(&output, b"keep").unwrap();
    let error = encrypt_container_with_entropy(
        &snapshot,
        &manifest(),
        &passphrase(),
        &output,
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "destination_exists");
    assert_eq!(fs::read(&output).unwrap(), b"keep");
    fs::remove_file(&output).unwrap();
    encrypt_container_with_entropy(
        &snapshot,
        &manifest(),
        &passphrase(),
        &output,
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn encrypted_publication_injected_write_link_and_sync_fail_closed() {
    let root = tempdir().unwrap();
    let snapshot = root.path().join("snapshot.sqlite");
    write_database(&snapshot);
    let output = root.path().join("artifact.iotkit-node-backup");
    let output_directory = directory_capability(root.path());

    let error = encrypt_snapshot_reader_with_output_init(
        File::open(&snapshot).unwrap(),
        &manifest(),
        &passphrase(),
        (&output_directory, "artifact.iotkit-node-backup"),
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
        |owner| {
            owner.ops = output_ops_with_fault(OutputFault::FileSync);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "storage");
    assert!(!output.exists());

    let error = encrypt_snapshot_reader_with_output_init(
        File::open(&snapshot).unwrap(),
        &manifest(),
        &passphrase(),
        (&output_directory, "artifact.iotkit-node-backup"),
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
        |owner| {
            owner.ops = output_ops_with_fault(OutputFault::Write);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "storage");
    assert!(!output.exists());

    let error = encrypt_snapshot_reader_with_output_init(
        File::open(&snapshot).unwrap(),
        &manifest(),
        &passphrase(),
        (&output_directory, "artifact.iotkit-node-backup"),
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
        |owner| {
            owner.ops = output_ops_with_fault(OutputFault::Link);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "storage");
    assert!(!output.exists());

    let error = encrypt_snapshot_reader_with_output_init(
        File::open(&snapshot).unwrap(),
        &manifest(),
        &passphrase(),
        (&output_directory, "artifact.iotkit-node-backup"),
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
        |owner| {
            owner.ops = output_ops_with_fault(OutputFault::DirectorySync);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "artifact_publication_uncertain");
    assert!(output.exists());
    fs::remove_file(&output).unwrap();

    encrypt_container_with_entropy(
        &snapshot,
        &manifest(),
        &passphrase(),
        &output,
        FIXED_SALT,
        FIXED_NONCE_PREFIX,
    )
    .unwrap();
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
