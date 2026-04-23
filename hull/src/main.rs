//! Hull — Generic Vesl NockApp. Fork this.
//!
//! Boots the settle kernel (1.5MB, no STARK prover) and serves
//! three endpoints: /commit, /settle, /verify.
//!
//! Community developers: add domain-specific endpoints, replace
//! FieldVerifier, and ship a NockApp for your computation domain.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use nockapp::kernel::boot;
use nockapp::NockApp;
use tokio::sync::Mutex;

use hull::api;
use hull::config::{self, SettlementMode};

#[derive(Parser)]
#[command(name = "hull", about = "Vesl Generic Hull -- fork this")]
#[group(id = "hull_cli")]
struct Cli {
    #[command(flatten)]
    boot: boot::Cli,

    /// Directory to persist state files. Defaults to current directory.
    #[arg(long = "output", default_value = ".")]
    output_dir: PathBuf,

    /// Port for the HTTP API server.
    #[arg(long = "port", default_value = "3000")]
    port: u16,

    /// Bind address for the HTTP API server.
    /// Use 0.0.0.0 to expose to the network.
    #[arg(long = "bind-addr", default_value = "127.0.0.1")]
    bind_addr: String,

    /// Settlement mode: local (default), fakenet, or dumbnet.
    #[arg(long = "settlement-mode", value_enum)]
    settlement_mode: Option<SettlementMode>,

    /// Path to vesl.toml config file.
    #[arg(long = "config", default_value = "../vesl.toml")]
    config: PathBuf,

    /// Nockchain gRPC endpoint for on-chain settlement.
    /// If set without --settlement-mode, infers fakenet.
    #[arg(long = "chain-endpoint")]
    chain_endpoint: Option<String>,

    /// Submit settlement transaction on-chain.
    /// If set without --settlement-mode, infers fakenet.
    #[arg(long = "submit")]
    submit: bool,

    /// Coinbase timelock minimum for UTXO spending [default: 1].
    #[arg(long = "coinbase-timelock-min")]
    coinbase_timelock_min: Option<u64>,

    /// Transaction fee in nicks [default: 3000].
    #[arg(long = "tx-fee")]
    tx_fee: Option<u64>,

    /// TX acceptance timeout in seconds [fakenet: 300, dumbnet: 900].
    #[arg(long = "accept-timeout")]
    accept_timeout: Option<u64>,

    /// Seed phrase for dumbnet key derivation.
    #[arg(long = "seed-phrase")]
    seed_phrase: Option<String>,

    /// Path to a file containing the seed phrase (one line, trimmed).
    #[arg(long = "seed-phrase-file")]
    seed_phrase_file: Option<PathBuf>,

    /// Disable API key authentication (local dev only).
    /// Without this flag, HULL_API_KEY must be set or the server refuses to start.
    #[arg(long = "no-auth")]
    no_auth: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // --- Load config ---
    let toml_cfg = config::load_config(&cli.config);

    // --- Resolve seed phrase: file > CLI arg > env ---
    let seed_phrase = if let Some(ref path) = cli.seed_phrase_file {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read seed phrase file {}: {e}", path.display()))?;
        Some(contents.trim().to_string())
    } else {
        cli.seed_phrase.clone()
    };

    if cli.seed_phrase.is_some() && cli.seed_phrase_file.is_none() {
        eprintln!("WARNING: --seed-phrase is visible in `ps` output. Use --seed-phrase-file instead.");
    }

    // --- Resolve settlement config (L-14: surface errors, don't panic) ---
    let settlement = config::resolve_with_demo_key_checked(
        cli.settlement_mode,
        cli.chain_endpoint.clone(),
        cli.submit,
        cli.tx_fee,
        cli.coinbase_timelock_min,
        cli.accept_timeout,
        seed_phrase,
        &toml_cfg,
    )
    .map_err(|e| {
        eprintln!("ERROR: settlement config: {e}");
        e
    })?;

    println!("=== Hull (Generic Vesl NockApp) ===\n");
    println!("    Settlement: {settlement}");

    // --- Boot the settle kernel (no STARK prover jets) ---
    println!("[0] Booting settle kernel...");
    // AUDIT 2026-04-17 M-07: verify the embedded JAM matches the
    // build-time sha256 before handing it to nockapp. Panics on
    // mismatch rather than booting a tampered kernel.
    kernels_settle::verify_kernel();
    let app: NockApp = boot::setup(
        kernels_settle::KERNEL,
        cli.boot,
        &[], // no prover jets -- settle kernel has no STARK
        "hull",
        None,
    )
    .await?;
    println!("    Kernel booted ({} bytes JAM)", kernels_settle::KERNEL.len());

    // C-004 / M-15: require auth config, and refuse --no-auth when
    // the bind address isn't loopback.
    api::check_auth_config_with_bind(cli.no_auth, &cli.bind_addr).map_err(|e| {
        eprintln!("ERROR: {e}");
        e
    })?;
    if cli.no_auth {
        eprintln!("WARNING: --no-auth passed. API key authentication is DISABLED.");
        eprintln!("         Do not use in production.");
    }

    // --- Start HTTP server ---
    let state = Arc::new(Mutex::new(api::AppState {
        app,
        fields: Vec::new(),
        tree: None,
        hull_id: 1,
        note_counter: api::load_note_counter(&cli.output_dir),
        settlement,
        output_dir: cli.output_dir,
    }));

    api::serve(state, cli.port, &cli.bind_addr).await
}
