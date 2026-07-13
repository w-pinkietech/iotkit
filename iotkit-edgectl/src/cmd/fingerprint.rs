use std::path::Path;

use rusqlite::Connection;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

pub fn run_fingerprint(_conn: &Connection, db_path: &Path) -> AppResult<()> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let cert_path = data_dir.join("tls").join("cert.pem");
    if !cert_path.exists() {
        return Err("未生成（Edge 未起動）".into());
    }

    let pem = std::fs::read_to_string(cert_path)?;
    println!("{}", iotkit_core_ops::fingerprint_of_pem(&pem)?);
    Ok(())
}
