//! `nodectl pipeline`: list, export, import, and reset device-local pipelines.
//!
//! Mutations go through the `pipeline.*` typed operations. After a committed
//! change the definitions are written to `pipelines.toml` (next to the
//! database unless `--export-path` says otherwise), the same backup the node
//! maintains.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use iotkit_core_ops::{Actor, ActorKind, DispatchRequest, Tier};
use rusqlite::Connection;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Subcommand)]
pub enum PipelineCommand {
    /// Print every pipeline definition as JSON.
    List,
    /// Write every definition to `pipelines.toml` (or `--export-path`).
    Export(ExportArgs),
    /// Replace every definition with the file's contents. Every pipeline
    /// starts a new series; pipelines missing from the file are deleted.
    Import(ImportArgs),
    /// Start a new series for one pipeline and clear its evaluation state.
    Reset(ResetArgs),
    /// Apply the definitions in the file to existing pipelines one by one.
    /// Tuning changes keep the series; structural changes start a new one.
    Update(UpdateArgs),
    /// Delete one pipeline and clear its retained value at the Broker.
    Delete(DeleteArgs),
}

#[derive(Args)]
pub struct ExportArgs {
    #[arg(long)]
    pub export_path: Option<PathBuf>,
}

#[derive(Args)]
pub struct ImportArgs {
    pub file: PathBuf,
    /// Required: import replaces every definition and restarts every series.
    #[arg(long)]
    pub replace_all: bool,
    #[arg(long)]
    pub export_path: Option<PathBuf>,
}

#[derive(Args)]
pub struct ResetArgs {
    pub id: String,
    #[arg(long)]
    pub export_path: Option<PathBuf>,
}

#[derive(Args)]
pub struct UpdateArgs {
    /// A `pipelines.toml`-shaped file; every `[[pipeline]]` in it is updated.
    pub file: PathBuf,
    #[arg(long)]
    pub export_path: Option<PathBuf>,
}

#[derive(Args)]
pub struct DeleteArgs {
    pub id: String,
    #[arg(long)]
    pub export_path: Option<PathBuf>,
}

pub fn run(conn: &Connection, db_path: &Path, command: PipelineCommand) -> AppResult<()> {
    match command {
        PipelineCommand::List => {
            let definitions = iotkit_core_pipeline::store::list_definitions(conn)?;
            println!("{}", serde_json::to_string_pretty(&definitions)?);
            Ok(())
        }
        PipelineCommand::Export(args) => {
            let path = export_path(db_path, args.export_path);
            iotkit_core_pipeline::export_definitions(conn, &path)?;
            println!("{}", path.display());
            Ok(())
        }
        PipelineCommand::Import(args) => {
            if !args.replace_all {
                return Err(
                    "import replaces every pipeline definition and restarts every series; pass --replace-all to confirm"
                        .into(),
                );
            }
            let definitions = iotkit_core_pipeline::read_definitions(&args.file)?;
            let result = dispatch(
                conn,
                iotkit_core_ops::ops::pipeline_ops::IMPORT,
                serde_json::json!({ "pipelines": definitions }),
                true,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            export_after_commit(conn, db_path, args.export_path)
        }
        PipelineCommand::Reset(args) => {
            let result = dispatch(
                conn,
                iotkit_core_ops::ops::pipeline_ops::RESET,
                serde_json::json!({ "id": args.id }),
                false,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            export_after_commit(conn, db_path, args.export_path)
        }
        PipelineCommand::Update(args) => {
            let definitions = iotkit_core_pipeline::read_definitions(&args.file)?;
            let mut results = Vec::with_capacity(definitions.len());
            for definition in definitions {
                results.push(dispatch(
                    conn,
                    iotkit_core_ops::ops::pipeline_ops::UPDATE,
                    serde_json::json!({ "definition": definition }),
                    false,
                )?);
            }
            println!("{}", serde_json::to_string_pretty(&results)?);
            export_after_commit(conn, db_path, args.export_path)
        }
        PipelineCommand::Delete(args) => {
            let result = dispatch(
                conn,
                iotkit_core_ops::ops::pipeline_ops::DELETE,
                serde_json::json!({ "id": args.id }),
                false,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            export_after_commit(conn, db_path, args.export_path)
        }
    }
}

fn dispatch(
    conn: &Connection,
    op: &str,
    params: serde_json::Value,
    step_up_verified: bool,
) -> AppResult<serde_json::Value> {
    let result = iotkit_core_ops::dispatch(
        conn,
        iotkit_core_ops::standard_catalog(),
        DispatchRequest {
            op: op.into(),
            params,
            dry_run: false,
            actor: Actor {
                actor_id: "local_cli".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: Some("local_cli".into()),
            step_up_verified,
            clock_trust: None,
        },
    )?;
    Ok(result.into_public()?)
}

fn export_path(db_path: &Path, override_path: Option<PathBuf>) -> PathBuf {
    override_path.unwrap_or_else(|| iotkit_core_pipeline::default_export_path(db_path))
}

/// The definition change has committed; a failed export is reported but does
/// not undo it, matching the node's behavior.
fn export_after_commit(
    conn: &Connection,
    db_path: &Path,
    override_path: Option<PathBuf>,
) -> AppResult<()> {
    let path = export_path(db_path, override_path);
    match iotkit_core_pipeline::export_definitions(conn, &path) {
        Ok(()) => Ok(()),
        Err(error) => Err(format!(
            "definitions committed, but exporting {} failed: {error}",
            path.display()
        )
        .into()),
    }
}
