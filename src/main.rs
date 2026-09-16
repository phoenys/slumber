mod cli;
mod core;
mod daemon;
mod resumer;
mod supervisor;
mod tmux;

use anyhow::Result;
use clap::Parser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    cli::Cli::parse().execute().await
}
