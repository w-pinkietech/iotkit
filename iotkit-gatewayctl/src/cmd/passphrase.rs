use std::io;

use clap::Subcommand;
use rusqlite::{Connection, TransactionBehavior};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Subcommand)]
pub enum PassphraseCommand {
    Reset,
}

pub fn run_passphrase_reset(conn: &Connection) -> AppResult<()> {
    let passphrase = read_confirmed_passphrase()?;
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    iotkit_core_ops::reset_passphrase(&tx, &passphrase, "local_cli")?;
    tx.commit()?;
    println!("passphrase reset");
    Ok(())
}

fn read_confirmed_passphrase() -> AppResult<String> {
    let first = read_line("new passphrase: ")?;
    if first.len() < 8 {
        return Err("passphrase must be at least 8 characters".into());
    }
    let second = read_line("confirm passphrase: ")?;
    if first != second {
        return Err("passphrases do not match".into());
    }
    Ok(first)
}

fn read_line(prompt: &str) -> io::Result<String> {
    println!("{prompt}");
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}
