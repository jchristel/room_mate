//! The `drofus-mcp` binary: parse the flags, then serve the tools over stdio.
//!
//! Logs go to stderr, never stdout: stdout is the JSON-RPC transport.

use clap::Parser;
use drofus_mcp::{
    approval::{Gate, GateConfig, DEFAULT_TTL},
    config::Config,
    server::DrofusMcp,
};
use rmcp::{transport::stdio, ServiceExt};

#[derive(Parser)]
#[command(version, about = "An MCP server for dRofus")]
struct Args {
    /// The dRofus API's base address, for example https://api-no.drofus.com. Falls back to the DROFUS_BASE_URL environment variable.
    #[arg(long)]
    base_url: Option<String>,

    /// The database every request is made against (`/api/{db}/{pr}/...`). Falls back to DROFUS_DATABASE;
    /// a tool's `db` argument overrides it.
    #[arg(long)]
    database: Option<String>,

    /// The project every request is made against. Falls back to DROFUS_PROJECT; a tool's `pr` argument
    /// overrides it.
    #[arg(long)]
    project: Option<String>,

    /// A credential slot that nothing reads yet: how dRofus authenticates is undecided (see AUTH.md). Prefer the DROFUS_API_KEY environment variable: a
    /// flag shows up in the process list.
    #[arg(long)]
    api_key: Option<String>,

    /// Turn on the approval gate that any write tool must pass. Off by default. There is no write tool yet,
    /// so today this changes nothing but what `server_info` says.
    #[arg(long, env = "DROFUS_ALLOW_WRITES")]
    allow_writes: bool,

    /// Where the gate appends its audit log (one JSON line per step; summary and hash, never the payload).
    /// Required with --allow-writes: a gate that cannot be audited is not started.
    #[arg(long, env = "DROFUS_AUDIT_LOG")]
    audit_log: Option<std::path::PathBuf>,
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
    let config =
        Config::resolve(args.base_url, args.api_key, args.database, args.project, |name| std::env::var(name).ok());
    for missing in config.missing() {
        tracing::warn!("not configured yet: {missing}");
    }

    if args.allow_writes && args.audit_log.is_none() {
        anyhow::bail!(
            "--allow-writes needs --audit-log (or DROFUS_AUDIT_LOG): a gate that cannot be audited is not started"
        );
    }
    let gate = Gate::new(GateConfig {
        enabled: args.allow_writes,
        audit_log: args.audit_log,
        ttl: DEFAULT_TTL,
        open_browser: true,
    });

    let service = DrofusMcp::new(config).with_gate(gate).serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {e:?}");
    })?;
    service.waiting().await?;
    Ok(())
}
