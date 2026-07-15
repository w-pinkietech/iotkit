use clap::{Args, Subcommand};
use iotkit_core_ops::{Actor, ActorKind, DispatchRequest, Tier};
use iotkit_core_publish::store::target_get;
use rusqlite::Connection;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Subcommand)]
pub enum SmokeCommand {
    Enqueue,
    Status(StatusArgs),
}

#[derive(Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub ledger_epoch: String,
    #[arg(long)]
    pub pub_seq: i64,
}

pub fn run_enqueue(conn: &Connection) -> AppResult<()> {
    let result = iotkit_core_ops::dispatch(
        conn,
        iotkit_core_ops::standard_catalog(),
        DispatchRequest {
            op: "exit.commissioning_smoke.enqueue".into(),
            params: serde_json::json!({}),
            dry_run: false,
            actor: Actor {
                actor_id: "local_cli".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: Some("local_cli".into()),
            step_up_verified: false,
            clock_trust: None,
        },
    )?;
    println!("{}", serde_json::to_string_pretty(result.metadata())?);
    Ok(())
}

pub fn run_status(
    conn: &Connection,
    identity: &iotkit_core_ledger::EdgeIdentity,
    args: StatusArgs,
) -> AppResult<()> {
    if args.pub_seq < 1 {
        return Err("pub_seq must be positive".into());
    }
    if args.ledger_epoch != identity.ledger_epoch {
        return Err("cannot determine smoke delivery after the Edge ledger epoch changed".into());
    }
    let target = target_get(conn)?.ok_or("MQTT Site target is not initialized")?;
    if target.target_id != "site"
        || !target.archive_responsible
        || !target.credential_token.is_empty()
    {
        return Err("configured exit target is not the MQTT Site target".into());
    }
    let accepted_through = if target.cursor_epoch.as_deref() == Some(args.ledger_epoch.as_str()) {
        target.cursor_pub_seq
    } else {
        0
    };
    let status = if accepted_through >= args.pub_seq {
        "delivered"
    } else {
        "pending"
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "target_id": "site",
            "ledger_epoch": args.ledger_epoch,
            "pub_seq": args.pub_seq,
            "accepted_through": accepted_through,
            "status": status,
        }))?
    );
    Ok(())
}
