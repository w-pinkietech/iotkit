use clap::Parser;
use iotkit_edge::{cli::Cli, lifecycle::ExitReason};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let exit = match iotkit_edge::cli::run(cli).await {
        Ok(exit) => exit,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    if !matches!(exit, ExitReason::Requested) {
        std::process::exit(1);
    }
}
