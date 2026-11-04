//! The `drofus-mcp` binary: parse the flags, then serve the tools over stdio.
//!
//! Logs go to stderr, never stdout: stdout is the JSON-RPC transport.

use std::sync::Arc;

use clap::Parser;
use drofus_mcp::{
    approval::{Gate, GateConfig, WebApprover, DEFAULT_TTL},
    config::Config,
    server::DrofusMcp,
};
use rmcp::{transport::stdio, ServiceExt};

#[derive(Parser)]
#[command(version, about = "An MCP server for dRofus")]
struct Args {
    /// The dRofus API's base address, for example https://api-no.drofus.com or your organisation's own host (a whole request address is accepted too: the host, database and project are taken from it). Falls back to the DROFUS_BASE_URL environment variable.
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

    /// The dRofus API key (from the Power Query credentials dialog), sent as `Authorization: Reference <key>`.
    /// Deprecated: a flag shows up in the process list. Use DROFUS_API_KEY_FILE or DROFUS_API_KEY.
    #[arg(long)]
    api_key: Option<String>,

    /// A file holding the API key (one line). Lets the MCP host's own configuration hold a path instead of
    /// the key. Used only when neither --api-key nor DROFUS_API_KEY is set. Keep it readable by you alone.
    #[arg(long, env = "DROFUS_API_KEY_FILE")]
    api_key_file: Option<String>,

    /// Seconds one request may take (1 to 600; default 60). Raise it for the slow endpoints, such as a table's change
    /// log. Falls back to DROFUS_TIMEOUT_SECS.
    #[arg(long, env = "DROFUS_TIMEOUT_SECS", value_parser = clap::value_parser!(u64).range(1..=600))]
    timeout: Option<u64>,

    /// Turn on the approval gate that any write tool must pass. Off by default. There is no write tool yet,
    /// so today this changes nothing but what `server_info` says.
    #[arg(long, env = "DROFUS_ALLOW_WRITES")]
    allow_writes: bool,

    /// Where the gate appends its audit log (one JSON line per step; summary and hash, never the payload).
    /// Required with --allow-writes: a gate that cannot be audited is not started.
    #[arg(long, env = "DROFUS_AUDIT_LOG")]
    audit_log: Option<std::path::PathBuf>,
}

/// A key file other accounts can read is no better than the plain-text host configuration it replaces, so say so.
/// A warning and not a refusal: a person who made the file by hand should be told, not locked out.
#[cfg(unix)]
fn warn_if_key_file_is_shared(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path)
        && meta.permissions().mode() & 0o077 != 0
    {
        tracing::warn!("the API key file {path} can be read by other accounts; run: chmod 600 {path}");
    }
}

#[cfg(not(unix))]
fn warn_if_key_file_is_shared(_path: &str) {}

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
    if args.api_key.is_some() {
        tracing::warn!(
            "--api-key is deprecated: it is visible in the process list. Use DROFUS_API_KEY_FILE or DROFUS_API_KEY"
        );
    }
    let mut config =
        Config::resolve(args.base_url, args.api_key, args.database, args.project, |name| std::env::var(name).ok());
    if let Some(path) = args.api_key_file.as_deref() {
        warn_if_key_file_is_shared(path);
    }
    config = match config.clone().with_key_file(args.api_key_file.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("{e}");
            config
        }
    };
    config.timeout_secs = args.timeout;
    for missing in config.missing() {
        tracing::warn!("not configured yet: {missing}");
    }

    if args.allow_writes && args.audit_log.is_none() {
        anyhow::bail!(
            "--allow-writes needs --audit-log (or DROFUS_AUDIT_LOG): a gate that cannot be audited is not started"
        );
    }
    let gate = Gate::new(
        GateConfig { enabled: args.allow_writes, audit_log: args.audit_log, ttl: DEFAULT_TTL },
        Arc::new(WebApprover::new(true)),
    );

    let service = DrofusMcp::new(config).with_gate(gate).serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {e:?}");
    })?;
    service.waiting().await?;
    Ok(())
}
