//! `cargo xtask` entry point: CLI parsing, tracing setup, and exit codes.

use clap::Parser;

/// CLI definition. Clap provides `--help` for the tool and every subcommand.
#[derive(Parser, Debug)]
#[command(
    name = "xtask",
    version,
    about = "rblue build and packaging orchestration (binding generation, distribution)"
)]
struct Xtask {
    /// The command to run; see the subcommand help for details.
    #[command(subcommand)]
    command: xtask::Command,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let Xtask { command } = Xtask::parse();
    if let Err(error) = xtask::run(command) {
        tracing::error!("{error:#}");
        std::process::exit(1);
    }
}
