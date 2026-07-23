use std::{path::PathBuf, process::Command as ProcessCommand};

use clap::Parser;
use iotkit_edge::{
    cli::{BackupCommand, Cli, Command},
    storage::{Storage, StorageProfile},
};
use tempfile::TempDir;

#[test]
fn clap_preserves_backup_and_diagnose_operational_flags() {
    let create = Cli::try_parse_from([
        "iotkit-edge",
        "backup",
        "create",
        "--db",
        "/data/edge.db",
        "--output",
        "/backup/edge.iotkit-backup",
        "--passphrase-file",
        "/run/secrets/backup",
    ])
    .unwrap();
    match create.command.unwrap() {
        Command::Backup {
            command: BackupCommand::Create(args),
        } => {
            assert_eq!(args.storage.database, PathBuf::from("/data/edge.db"));
            assert_eq!(args.output, PathBuf::from("/backup/edge.iotkit-backup"));
            assert_eq!(args.passphrase_file, PathBuf::from("/run/secrets/backup"));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    Cli::try_parse_from([
        "iotkit-edge",
        "diagnose",
        "--storage-profile",
        "postgres",
        "--postgres-config",
        "/run/secrets/postgres.json",
        "--storage-metadata",
        "/run/iotkit/storage-profile.json",
    ])
    .expect("parse PostgreSQL diagnostics");
}

#[test]
fn secrets_are_file_inputs_and_cannot_be_supplied_as_cli_values() {
    assert!(
        Cli::try_parse_from([
            "iotkit-edge",
            "backup",
            "create",
            "--db",
            "edge.db",
            "--output",
            "backup",
            "--passphrase",
            "must-not-appear",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "iotkit-edge",
            "diagnose",
            "--storage-profile",
            "postgres",
            "--postgres-dsn",
            "postgres://user:secret@host/db",
        ])
        .is_err()
    );
}

#[test]
fn serve_accepts_every_current_deployment_flag_without_secret_value_flags() {
    Cli::try_parse_from([
        "iotkit-edge",
        "serve",
        "--storage-profile",
        "postgres",
        "--postgres-config",
        "/run/iotkit/postgres.json",
        "--storage-metadata",
        "/run/iotkit/storage-profile.json",
        "--edge-id",
        "edge-id",
        "--broker-url",
        "ssl://broker:8883",
        "--client-id",
        "edge-client",
        "--username",
        "edge-user",
        "--password-file",
        "/run/iotkit/mqtt-password",
        "--trust-mode",
        "bundle_only",
        "--ca-file",
        "/run/iotkit/ca.pem",
        "--http-listen",
        "127.0.0.1:8080",
        "--public-origin",
        "https://edge.example",
        "--development-http",
        "--broker-certificate-file",
        "/run/iotkit/server.pem",
        "--storage-warning-percent",
        "90",
        "--output-broker-url",
        "ssl://output:8883",
        "--output-client-id",
        "output-client",
        "--output-username",
        "output-user",
        "--output-password-file",
        "/run/iotkit/output-password",
        "--output-trust-mode",
        "bundle_only",
        "--output-ca-file",
        "/run/iotkit/output-ca.pem",
        "--output-allow-insecure",
    ])
    .unwrap();
    assert!(
        Cli::try_parse_from([
            "iotkit-edge",
            "serve",
            "--edge-id",
            "edge",
            "--broker-url",
            "ssl://broker",
            "--username",
            "user",
            "--password",
            "forbidden",
            "--public-origin",
            "https://edge.example",
        ])
        .is_err()
    );
}

#[test]
fn no_arguments_prints_usage_to_stderr_and_exits_nonzero() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_iotkit-edge"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("usage: iotkit-edge"));
}

#[tokio::test]
async fn diagnose_exits_zero_with_json_only_on_stdout() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    drop(storage);

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_iotkit-edge"))
        .args(["diagnose", "--db", database.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["state"], "attention");
    assert!(report["generated_at"].as_i64().is_some());
}
