use super::*;
use crate::definition::tests::{count_definition, measurement_definition};
use crate::engine::PipelineEngine;

fn open() -> iotkit_core_storage::DbHandle {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(crate::MIGRATIONS);
    iotkit_core_storage::init_db_memory(&migrations).unwrap()
}

#[test]
fn rendered_toml_round_trips_every_definition() {
    let definitions = vec![count_definition(), measurement_definition()];
    let rendered = render_definitions(&definitions);
    assert!(rendered.contains("[[pipeline]]"));
    assert!(rendered.contains("kind = \"accumulated-count\""));
    assert!(rendered.contains("[pipeline.input]"));
    assert!(rendered.contains("[pipeline.detector]"));
    assert!(rendered.contains("mode = \"high-active\""));
    assert_eq!(parse_definitions(&rendered).unwrap(), definitions);
    assert!(parse_definitions("").unwrap().is_empty());
    assert!(parse_definitions("[[pipeline]]\nid = \"x\"\nkind = \"nope\"\n").is_err());
    assert!(
        parse_definitions("[[pipelines]]\n").is_err(),
        "unknown table"
    );
}

#[test]
fn export_writes_the_stored_definitions_atomically() {
    let db = open();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pipelines.toml");
    let engine = PipelineEngine::new("rpi1".parse().unwrap());
    db.with_conn_sync(|conn| {
        engine.create(conn, &count_definition(), 0).unwrap();
        export_definitions(conn, &path).unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(read_definitions(&path).unwrap(), vec![count_definition()]);
    assert!(
        !directory.path().join("pipelines.toml.tmp").exists(),
        "temporary file is renamed away"
    );

    db.with_conn_sync(|conn| {
        engine.create(conn, &measurement_definition(), 1).unwrap();
        export_definitions(conn, &path).unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        read_definitions(&path).unwrap(),
        vec![count_definition(), measurement_definition()]
    );

    let missing_dir = directory.path().join("missing").join("pipelines.toml");
    let error = db
        .with_conn_sync(|conn| Ok(export_definitions(conn, &missing_dir)))
        .unwrap()
        .unwrap_err();
    assert!(matches!(error, ExportError::Io { .. }));
    assert!(matches!(
        read_definitions(&missing_dir).unwrap_err(),
        ImportError::Io { .. }
    ));
}
