use clap::{Parser, Subcommand};
use iotkit_edge::{Application, lifecycle::ExitReason};

#[derive(Debug, Parser)]
#[command(name = "iotkit-edge", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the IoTKit Edge server.
    Serve,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let exit = match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => Application::new().run().await,
    };

    if !matches!(exit, ExitReason::Requested) {
        std::process::exit(1);
    }
}
