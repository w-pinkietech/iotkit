use clap::{Args, Subcommand, ValueEnum};
use rusqlite::{Connection, TransactionBehavior};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Subcommand)]
pub enum TokenCommand {
    Issue(IssueArgs),
    Revoke(RevokeArgs),
    List,
}

#[derive(Args)]
pub struct IssueArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long, value_enum)]
    pub kind: TokenKindArg,
    #[arg(long, value_enum)]
    pub tier: TierArg,
}

#[derive(Args)]
pub struct RevokeArgs {
    #[arg(long)]
    pub id: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TokenKindArg {
    Human,
    Ai,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum TierArg {
    ReadOnly,
    Routine,
    Daily,
    Construction,
}

pub fn run_token_issue(conn: &Connection, args: IssueArgs) -> AppResult<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let issued = iotkit_core_ops::issue_token(
        &tx,
        &iotkit_core_ops::NewOperatorToken {
            name: args.name,
            kind: args.kind.into(),
            ceiling: args.tier.into(),
            is_session: false,
            expires_at: None,
        },
        "local_cli",
        None,
    )?;
    tx.commit()?;
    eprintln!("token_id: {}", issued.token_id);
    println!("{}", issued.plaintext.expose());
    Ok(())
}

pub fn run_token_revoke(conn: &Connection, args: RevokeArgs) -> AppResult<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    iotkit_core_ops::revoke_token(&tx, &args.id, "local_cli")?;
    tx.commit()?;
    Ok(())
}

pub fn run_token_list(conn: &Connection) -> AppResult<()> {
    println!("token_id\tname\tkind\ttier_ceiling\tis_session\texpires_at\trevoked_at\tlast_used");
    for row in iotkit_core_ops::list_tokens(conn)? {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.token_id,
            row.name,
            row.kind.as_str(),
            row.tier_ceiling.as_str(),
            row.is_session,
            format_optional_ms(row.expires_at),
            format_optional_ms(row.revoked_at),
            format_optional_ms(row.last_used_at),
        );
    }
    Ok(())
}

fn format_optional_ms(value: Option<i64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

impl From<TokenKindArg> for iotkit_core_ops::TokenKind {
    fn from(value: TokenKindArg) -> Self {
        match value {
            TokenKindArg::Human => Self::Human,
            TokenKindArg::Ai => Self::Ai,
        }
    }
}

impl From<TierArg> for iotkit_core_ops::Tier {
    fn from(value: TierArg) -> Self {
        match value {
            TierArg::ReadOnly => Self::ReadOnly,
            TierArg::Routine => Self::Routine,
            TierArg::Daily => Self::Daily,
            TierArg::Construction => Self::Construction,
        }
    }
}
