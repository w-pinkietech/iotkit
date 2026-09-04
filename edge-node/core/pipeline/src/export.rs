//! `pipelines.toml`: a backup of the definitions derived from the database
//! after every committed change. Never read at startup; restoring it is the
//! explicit `nodectl pipeline import`.

use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::definition::PipelineDefinition;
use crate::store::{self, StoreError};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportFile {
    #[serde(default, rename = "pipeline")]
    pipelines: Vec<PipelineDefinition>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("failed to write {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
}

pub const DEFAULT_EXPORT_FILE_NAME: &str = "pipelines.toml";

/// `pipelines.toml` next to the database, the default of `[pipelines] export_path`.
pub fn default_export_path(db_path: &Path) -> PathBuf {
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(DEFAULT_EXPORT_FILE_NAME),
        _ => PathBuf::from(DEFAULT_EXPORT_FILE_NAME),
    }
}

/// Renders the definitions as the TOML document written to `pipelines.toml`.
pub fn render_definitions(definitions: &[PipelineDefinition]) -> String {
    let file = ExportFile {
        pipelines: definitions.to_vec(),
    };
    toml::to_string_pretty(&file).expect("definitions serialize to TOML")
}

/// Writes every stored definition to `path` atomically (temporary file in the
/// same directory, fsync, rename). Call after the definition change committed.
pub fn export_definitions(conn: &Connection, path: &Path) -> Result<(), ExportError> {
    let definitions = store::list_definitions(conn)?;
    let contents = render_definitions(&definitions);
    write_atomically(path, contents.as_bytes()).map_err(|source| ExportError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(".tmp");
    let temporary = path.with_file_name(temporary_name);
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Parses a `pipelines.toml`. Definitions are validated by the import
/// operation, not here.
pub fn read_definitions(path: &Path) -> Result<Vec<PipelineDefinition>, ImportError> {
    let contents = std::fs::read_to_string(path).map_err(|source| ImportError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_definitions(&contents).map_err(|source| ImportError::Toml {
        path: path.display().to_string(),
        source,
    })
}

pub fn parse_definitions(contents: &str) -> Result<Vec<PipelineDefinition>, toml::de::Error> {
    toml::from_str::<ExportFile>(contents).map(|file| file.pipelines)
}

#[cfg(test)]
#[path = "../tests/unit/export_tests.rs"]
mod tests;
