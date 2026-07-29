use iotkit_core_storage::{Migration, run_migrations};
use rusqlite::Connection;

pub fn complete_database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, &crate::all_edge_node_migrations()).unwrap();
    conn
}

pub fn pre_recovery_migrations() -> Vec<Migration> {
    let mut migrations = crate::all_edge_node_migrations();
    migrations.retain(|migration| migration.version < 23);
    migrations
}

pub fn assert_table_columns(conn: &Connection, table: &str, expected: &[&str]) {
    let actual: Vec<String> = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(actual, expected);
}
