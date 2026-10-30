//! The `drofus-mcp` binary: parse the flags, then serve the tools over stdio.
//!
//! Logs go to stderr, never stdout: stdout is the JSON-RPC transport.

use clap::Parser;
use drofus_mcp::{config::Config, server::DrofusMcp};
use rmcp::{transport::stdio, ServiceExt};

#[derive(Parser)]
#[command(version, about = "An MCP server for dRofus")]
struct Args {
    /// The dRofus server's base address. Falls back to the DROFUS_BASE_URL environment variable.
    #[arg(long)]
    base_url: Option<String>,

    /// The credential the dRofus API asks for. Prefer the DROFUS_API_KEY environment variable: a
    /// flag shows up in the process list.
    #[arg(long)]
    api_key: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("drofus_mcp=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let config = Config::resolve(args.base_url, args.api_key, |name| std::env::var(name).ok());
    for missing in config.missing() {
        tracing::warn!("not configured yet: {missing}");
    }

    let service = DrofusMcp::new(config).serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {e:?}");
    })?;
    service.waiting().await?;
    Ok(())
}
