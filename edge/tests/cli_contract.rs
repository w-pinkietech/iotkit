use std::path::PathBuf;

use clap::Parser;
use iotkit_edge::cli::{BackupCommand, Cli, Command};

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
