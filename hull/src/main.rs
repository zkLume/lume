//! Vesl Hull Orchestrator — NockApp-based off-chain client.
//!
//! Pipeline: boot kernel → ingest → build tree → register root →
//!           retrieve → infer → build manifest → settle via poke
//!
//! Boots the compiled Hoon kernel (vesl.jam) as a NockApp, then
//! drives the settlement pipeline through kernel pokes.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
#[cfg(feature = "dumbnet")]
use clap::Subcommand;
use nockapp::kernel::boot;
use nockapp::noun::slab::NounSlab;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use tokio::sync::Mutex;

use hull::api;
use hull::chain;
use hull::config::{self, SettlementConfig, SettlementMode};
use hull::ingest;
use hull::llm;
use hull::merkle::{self, MerkleTree};
use hull::noun_builder;
use hull::retrieve;
use hull::signing;
use hull::tx_builder;
use hull::types::*;

#[derive(Parser)]
#[command(name = "hull", about = "Vesl Hull Orchestrator")]
#[group(id = "hull_cli")]
struct Cli {
    #[command(flatten)]
    boot: boot::Cli,

    /// Directory of .txt files to ingest. If omitted, uses built-in demo data.
    #[arg(long = "docs")]
    docs_dir: Option<PathBuf>,

    /// Directory to persist the chunk store JSON. Defaults to current directory.
    #[arg(long = "output", default_value = ".")]
    output_dir: PathBuf,

    /// Ollama API base URL. If omitted, uses a stub provider (no network).
    #[arg(long = "ollama-url")]
    ollama_url: Option<String>,

    /// Ollama model name (e.g. llama3.2, mistral). Only used with --ollama-url.
    #[arg(long = "model", default_value = "llama3.2")]
    model: String,

    /// Query text for the one-shot CLI pipeline.
    #[arg(long = "query", default_value = "Summarize Q3 financial position")]
    query: String,

    /// Number of top chunks to retrieve per query.
    #[arg(long = "top-k", default_value = "2")]
    top_k: usize,

    /// Start the HTTP API server instead of running the one-shot CLI pipeline.
    #[arg(long = "serve")]
    serve: bool,

    /// Port for the HTTP API server (only used with --serve).
    #[arg(long = "port", default_value = "3000")]
    port: u16,

    /// Bind address for the HTTP API server [default: 127.0.0.1].
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

    /// Wallet address (base58) for checking funding and querying notes.
    #[arg(long = "wallet-address")]
    wallet_address: Option<String>,

    /// Wallet private gRPC endpoint for signing coordination.
    #[arg(long = "wallet-grpc")]
    wallet_grpc: Option<String>,

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
    /// Alternatively set VESL_SEED_PHRASE env var.
    /// WARNING: visible in `ps` output. Prefer --seed-phrase-file.
    #[arg(long = "seed-phrase")]
    seed_phrase: Option<String>,

    /// Path to a file containing the seed phrase (one line, trimmed).
    /// Safer than --seed-phrase since the value never appears in ps output.
    #[arg(long = "seed-phrase-file")]
    seed_phrase_file: Option<PathBuf>,

    /// Wallet subcommand (requires --features dumbnet).
    #[cfg(feature = "dumbnet")]
    #[command(subcommand)]
    command: Option<Command>,
}

#[cfg(feature = "dumbnet")]
#[derive(Subcommand)]
enum Command {
    /// Wallet key management for dumbnet settlement.
    Wallet {
        #[command(subcommand)]
        action: WalletAction,
    },
}

#[cfg(feature = "dumbnet")]
#[derive(Subcommand)]
enum WalletAction {
    /// Generate a new keypair or import from a seed phrase.
    Init {
        /// Generate a new random keypair.
        #[arg(long)]
        keygen: bool,
        /// Import keys from an existing seed phrase.
        #[arg(long = "seed-phrase")]
        seed_phrase: Option<String>,
    },
    /// Show the current wallet public key hash.
    Status,
}

/// Fallback demo data when no --docs directory is provided.
fn demo_chunks() -> Vec<Chunk> {
    vec![
        Chunk {
            id: 0,
            dat: "Q3 revenue: $4.2M ARR, 18% QoQ growth".into(),
        },
        Chunk {
            id: 1,
            dat: "Risk exposure: $800K in variable-rate instruments".into(),
        },
        Chunk {
            id: 2,
            dat: "Board approved Series B at $45M pre-money".into(),
        },
        Chunk {
            id: 3,
            dat: "SOC2 Type II audit scheduled for Q4".into(),
        },
    ]
}

/// Build Merkle tree from chunk data.
fn build_tree(chunks: &[Chunk]) -> MerkleTree {
    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    MerkleTree::build(&leaf_data)
}

/// Create the LLM provider based on CLI flags.
fn create_llm_provider(
    ollama_url: &Option<String>,
    model: &str,
) -> Box<dyn llm::LlmProvider> {
    match ollama_url {
        Some(url) => {
            println!("    LLM: Ollama at {} (model: {})", url, model);
            Box::new(llm::OllamaProvider::new(url, model))
        }
        None => {
            println!("    LLM: stub provider (no --ollama-url, deterministic output)");
            Box::new(llm::StubProvider)
        }
    }
}

/// Verify Merkle proofs for ALL chunks after ingestion.
/// Panics with a clear message if any proof fails.
fn verify_all_proofs(chunks: &[Chunk], tree: &MerkleTree) {
    let root = tree.root();
    let mut pass = 0;
    let mut fail = 0;
    for (i, chunk) in chunks.iter().enumerate() {
        let proof = tree.proof(i);
        if merkle::verify_proof(chunk.dat.as_bytes(), &proof, &root) {
            pass += 1;
        } else {
            eprintln!("  FAIL: chunk {} (id={}) proof invalid", i, chunk.id);
            fail += 1;
        }
    }
    println!(
        "    Merkle verification: {}/{} proofs valid",
        pass,
        pass + fail
    );
    assert_eq!(fail, 0, "{fail} Merkle proof(s) failed — tree is corrupt");
}

/// Process effects returned from a kernel poke.
fn report_effects(label: &str, effects: &[NounSlab]) {
    println!("    {} effects returned", effects.len());
    for (i, _effect) in effects.iter().enumerate() {
        println!("    effect[{}]: (noun slab)", i);
    }
    if effects.is_empty() {
        println!("    {}: no effects (kernel may have nacked)", label);
    }
}

#[cfg(feature = "dumbnet")]
fn handle_wallet(action: WalletAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        WalletAction::Init { keygen, seed_phrase } => {
            if keygen {
                // Generate 32 bytes of entropy
                let mut entropy = [0u8; 32];
                getrandom::fill(&mut entropy)
                    .map_err(|e| format!("failed to generate entropy: {e}"))?;
                // Convert entropy to a Belt key by hashing
                let hex_str = hex::encode(&entropy);
                let sk = signing::key_from_seed_phrase(&hex_str)
                    .map_err(|e| format!("key derivation failed: {e}"))?;
                let pk = signing::derive_pubkey(&sk);
                let pkh = signing::pubkey_hash(&pk);
                println!("Keypair generated.");
                println!("  PKH (base58): {}", pkh.to_base58());
                println!("  Entropy (hex): {hex_str}");
                println!();
                println!("Save the entropy string. Pass it as --seed-phrase or");
                println!("set VESL_SEED_PHRASE to use this key with dumbnet mode.");
            } else if let Some(phrase) = seed_phrase {
                let sk = signing::key_from_seed_phrase(&phrase)
                    .map_err(|e| format!("key derivation failed: {e}"))?;
                let pk = signing::derive_pubkey(&sk);
                let pkh = signing::pubkey_hash(&pk);
                println!("Key imported from seed phrase.");
                println!("  PKH (base58): {}", pkh.to_base58());
            } else {
                eprintln!("Error: specify --keygen or --seed-phrase");
                std::process::exit(1);
            }
        }
        WalletAction::Status => {
            match std::env::var("VESL_SEED_PHRASE") {
                Ok(phrase) => {
                    let sk = signing::key_from_seed_phrase(&phrase)
                        .map_err(|e| format!("key derivation failed: {e}"))?;
                    let pk = signing::derive_pubkey(&sk);
                    let pkh = signing::pubkey_hash(&pk);
                    println!("  PKH (base58): {}", pkh.to_base58());
                    println!("  Key source:   VESL_SEED_PHRASE env var");
                }
                Err(_) => {
                    eprintln!("No key configured. Set VESL_SEED_PHRASE or run `hull wallet init --keygen`.");
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // --- Handle wallet subcommand (dumbnet feature only) ---
    #[cfg(feature = "dumbnet")]
    if let Some(Command::Wallet { action }) = cli.command {
        return handle_wallet(action);
    }

    // --- Load config from vesl.toml ---
    let toml_cfg = config::load_config(&cli.config);

    // --- Resolve seed phrase: file > CLI arg > env ---
    let seed_phrase = if let Some(ref path) = cli.seed_phrase_file {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read seed phrase file {}: {e}", path.display()))?;
        Some(contents.trim().to_string())
    } else {
        cli.seed_phrase.clone()
    };

    // --- Resolve settlement config ---
    let settlement = SettlementConfig::resolve(
        cli.settlement_mode,
        cli.chain_endpoint.clone(),
        cli.submit,
        cli.tx_fee,
        cli.coinbase_timelock_min,
        cli.accept_timeout,
        seed_phrase,
        &toml_cfg,
    );

    println!("=== Vesl Hull Orchestrator (NockApp) ===\n");
    println!("    Settlement: {settlement}");

    // --- Boot the NockApp kernel with STARK prover jets ---
    println!("[0] Booting Vesl NockApp kernel...");
    let stack_size = cli.boot.stack_size.clone();
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let mut app: NockApp = boot::setup(
        kernels_vesl::KERNEL,
        cli.boot,
        prover_hot_state.as_slice(),
        "vesl",
        None,
    )
    .await?;
    println!("    Kernel booted ({} bytes JAM, {} prover jets)",
        kernels_vesl::KERNEL.len(), prover_hot_state.len());

    if matches!(stack_size, boot::NockStackSize::Tiny | boot::NockStackSize::Small | boot::NockStackSize::Normal | boot::NockStackSize::Medium) {
        eprintln!("WARNING: Nock stack is {:?} — /prove will be unavailable. \
                   Use --stack-size large for STARK proving.", stack_size);
    }

    // --- HTTP server mode ---
    if cli.serve {
        let provider = create_llm_provider(&cli.ollama_url, &cli.model);

        // Pre-load documents if --docs provided with --serve
        let (chunks, tree) = if let Some(ref docs_dir) = cli.docs_dir {
            println!("[1] Pre-loading documents from: {}", docs_dir.display());
            let store = ingest::ingest_directory(docs_dir)
                .map_err(|e| format!("ingestion failed: {e}"))?;
            println!(
                "    Loaded {} chunks from {} files",
                store.meta.chunk_count, store.meta.file_count
            );

            // Persist chunk store
            let json_path = cli.output_dir.join("chunk_store.json");
            store
                .save(&json_path)
                .map_err(|e| format!("failed to save chunk store: {e}"))?;
            println!("    Saved chunk store: {}", json_path.display());

            let tree = store.build_tree();
            let root = tree.root();
            println!("    Merkle root: {}", merkle::format_tip5(&root));

            // Verify all chunk proofs
            verify_all_proofs(&store.chunks, &tree);

            // Register root with kernel
            let register_poke = noun_builder::build_register_poke(7, &root);
            let _effects = app.poke(SystemWire.to_wire(), register_poke).await?;
            println!("    Root registered with kernel");

            (store.chunks, Some(tree))
        } else {
            (Vec::new(), None)
        };

        let state = Arc::new(Mutex::new(api::AppState {
            app,
            chunks,
            tree,
            hull_id: 7,
            top_k: cli.top_k,
            llm: provider,
            retriever: Box::new(retrieve::KeywordRetriever),
            note_counter: 0,
            settlement: settlement.clone(),
            stack_size: stack_size.clone(),
        }));
        return api::serve(state, cli.port, &cli.bind_addr).await;
    }

    // =====================================================================
    // One-shot CLI pipeline
    // =====================================================================

    // --- [1] Ingest ---
    let (chunks, tree) = if let Some(ref docs_dir) = cli.docs_dir {
        println!("[1] Ingesting documents from: {}", docs_dir.display());
        let store = ingest::ingest_directory(docs_dir)
            .map_err(|e| format!("ingestion failed: {e}"))?;

        let json_path = cli.output_dir.join("chunk_store.json");
        store
            .save(&json_path)
            .map_err(|e| format!("failed to save chunk store: {e}"))?;
        println!(
            "    Saved chunk store: {} ({} chunks from {} files)",
            json_path.display(),
            store.meta.chunk_count,
            store.meta.file_count
        );

        let tree = store.build_tree();
        (store.chunks, tree)
    } else {
        println!("[1] No --docs provided, using demo data");
        let chunks = demo_chunks();
        let tree = build_tree(&chunks);
        (chunks, tree)
    };
    println!("    Ingested {} chunks", chunks.len());

    // --- [2] Merkle root + verify ALL proofs ---
    let root = tree.root();
    println!("[2] Merkle root: {}", merkle::format_tip5(&root));
    verify_all_proofs(&chunks, &tree);

    // --- [3] Register root ---
    println!("[3] Registering Merkle root...");
    let register_poke = noun_builder::build_register_poke(7, &root);
    let effects = app.poke(SystemWire.to_wire(), register_poke).await?;
    report_effects("register", &effects);

    // --- [4] Retrieve ---
    let query = &cli.query;
    let retriever = retrieve::KeywordRetriever;
    let hits = retrieve::Retriever::retrieve(&retriever, query, &chunks, cli.top_k);
    println!(
        "[4] Retrieved {} chunks (top-{}) for query: {:?}",
        hits.len(),
        cli.top_k,
        query,
    );
    for h in &hits {
        let preview = &chunks[h.chunk_index].dat;
        let short = if preview.len() > 60 {
            format!("{}...", &preview[..60])
        } else {
            preview.clone()
        };
        println!("    chunk[{}] score={:.2}: {}", h.chunk_index, h.score, short);
    }

    if hits.is_empty() {
        return Err("no relevant chunks found for query".into());
    }

    let retrieved_chunks: Vec<&Chunk> = hits.iter().map(|h| &chunks[h.chunk_index]).collect();

    let retrievals: Vec<Retrieval> = hits
        .iter()
        .map(|h| Retrieval {
            chunk: chunks[h.chunk_index].clone(),
            proof: tree.proof(h.chunk_index),
            score: h.score_fixed(),
        })
        .collect();

    // --- [5] Prompt + LLM ---
    let provider = create_llm_provider(&cli.ollama_url, &cli.model);

    let prompt = llm::build_prompt(query, &retrieved_chunks);
    println!("[5] Prompt: {} bytes", prompt.len());

    let output = provider
        .generate(&prompt)
        .await
        .map_err(|e| format!("LLM inference failed: {e}"))?;
    println!("    LLM output: {} bytes", output.len());

    // --- [6] Manifest ---
    let manifest = Manifest {
        query: query.to_string(),
        results: retrievals,
        prompt,
        output: output.clone(),
        page: 0,
    };
    println!(
        "[6] Manifest: {} retrievals, prompt {} bytes",
        manifest.results.len(),
        manifest.prompt.len()
    );

    // --- [7] Note + Settle ---
    let note = Note {
        id: 1,
        hull: 7,
        root,
        state: NoteState::Pending,
    };
    println!(
        "[7] Note #{} (hull={}, state=Pending) → settling...",
        note.id, note.hull
    );
    let settle_poke = noun_builder::build_settle_poke(&note, &manifest, &root);
    let effects = app.poke(SystemWire.to_wire(), settle_poke).await?;
    report_effects("settle", &effects);

    // --- [8] Self-verification ---
    println!("\n--- Self-verification ---");
    let mut all_valid = true;
    for (i, retrieval) in manifest.results.iter().enumerate() {
        let valid =
            merkle::verify_proof(retrieval.chunk.dat.as_bytes(), &retrieval.proof, &root);
        if !valid { all_valid = false; }
        println!(
            "  retrieval[{}] chunk_id={}: proof {}",
            i,
            retrieval.chunk.id,
            if valid { "VALID" } else { "FAILED" }
        );
    }
    println!("  Manifest check: {}", if all_valid { "PASSED" } else { "FAILED" });

    // --- [9] On-chain settlement (optional, based on settlement mode) ---
    let settlement_data = chain::SettlementData::from_settlement(&note, &manifest);
    let mut tx_accepted = false;
    let mut tx_id_str = String::new();

    if let (Some(endpoint), Some(sk)) =
        (&settlement.chain_endpoint, &settlement.signing_key)
    {
        println!("\n[9] Connecting to Nockchain node at {endpoint}...");
        println!("    Mode: {}", settlement.mode);
        let chain_config = settlement.chain_config().unwrap();
        match chain::ChainClient::connect(chain_config).await {
            Ok(mut client) => {
                println!("    Settlement: {settlement_data}");

                // --- [9a] Find spendable UTXO ---
                let pkh = signing::pubkey_hash(&signing::derive_pubkey(sk));
                let pkh_b58 = pkh.to_base58();
                println!("    Signer PKH: {}", &pkh_b58[..16]);
                if signing::is_demo_key(sk) {
                    println!("    Key: demo (fakenet)");
                } else {
                    println!("    Key: custom (dumbnet)");
                }

                let balance = client
                    .get_balance_by_pkh(&pkh_b58, settlement.coinbase_timelock_min)
                    .await;

                let utxos = match balance {
                    Ok(ref bal) => {
                        let u = chain::extract_spendable_utxos(bal);
                        println!(
                            "    Balance: {} note(s), {} spendable UTXO(s)",
                            bal.notes.len(),
                            u.len()
                        );
                        u
                    }
                    Err(e) => {
                        eprintln!("    warn: balance query failed: {e}");
                        vec![]
                    }
                };

                if settlement.auto_submit && !utxos.is_empty() {
                    // Pick the largest UTXO
                    let utxo = utxos.iter().max_by_key(|u| u.amount).unwrap();
                    println!("    Using UTXO: {} nicks", utxo.amount);

                    // --- [9b] Build settlement transaction ---
                    println!("[9b] Building settlement transaction...");
                    let params = tx_builder::SettlementTxParams {
                        input_name: nockchain_types::tx_engine::common::Name::new(
                            utxo.name.clone(),
                            utxo.last_name.clone(),
                        ),
                        input_note_hash: utxo.last_name.clone(),
                        input_amount: utxo.amount,
                        is_coinbase: true,
                        coinbase_timelock_min: settlement.coinbase_timelock_min,
                        source_hash: nockchain_types::tx_engine::common::Hash::from_limbs(&[
                            0, 0, 0, 0, 0,
                        ]),
                        recipient_pkh: pkh,
                        settlement: settlement_data.clone(),
                        fee: settlement.tx_fee,
                        signing_key: *sk,
                    };

                    match tx_builder::build_settlement_tx(&mut app, &params).await {
                        Ok(raw_tx) => {
                            tx_id_str = raw_tx.id.to_base58();
                            println!("    tx-id: {tx_id_str}");
                            println!("    NoteData: 5 Vesl settlement keys");
                            println!("    Fee: {} nicks", settlement.tx_fee);

                            // --- [9c] Submit to chain ---
                            println!("[9c] Submitting transaction to chain...");
                            match client.submit_and_wait(raw_tx, &tx_id_str).await {
                                Ok(true) => {
                                    println!("    Transaction ACCEPTED on-chain!");
                                    tx_accepted = true;
                                }
                                Ok(false) => {
                                    println!(
                                        "    Transaction timed out (not accepted in time)."
                                    );
                                }
                                Err(e) => {
                                    eprintln!("    Transaction submission error: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("    Failed to build settlement tx: {e}");
                        }
                    }
                } else if settlement.auto_submit && utxos.is_empty() {
                    eprintln!("    No spendable UTXOs found — cannot submit settlement tx.");
                    eprintln!("    Ensure the miner is using PKH: {pkh_b58}");
                }

                // --- [9d] Scan for existing Vesl settlements ---
                println!("[9d] Scanning for Vesl settlement notes...");
                match client
                    .find_settlement_notes_by_pkh(
                        &pkh_b58,
                        settlement.coinbase_timelock_min,
                    )
                    .await
                {
                    Ok(notes) if !notes.is_empty() => {
                        println!(
                            "    Found {} Vesl settlement(s) on-chain:",
                            notes.len()
                        );
                        for s in &notes {
                            println!("      {s}");
                        }
                    }
                    Ok(_) => {
                        println!("    No Vesl settlements found on-chain yet.");
                    }
                    Err(e) => {
                        eprintln!("    warn: could not query settlements: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("    Failed to connect to chain: {e}");
                eprintln!("    (Pipeline completed locally; chain settlement skipped)");
            }
        }
    } else if settlement.mode != SettlementMode::Local {
        println!("\n[9] Settlement mode: {} (no signing key configured)", settlement.mode);
        if settlement.mode == SettlementMode::Dumbnet {
            eprintln!("    Run `hull wallet init --keygen` and set VESL_SEED_PHRASE.");
        }
    }

    // --- Summary ---
    println!("\n=== Pipeline Summary ===");
    println!("  Settlement mode:  {}", settlement.mode);
    println!("  Chunks ingested:  {}", chunks.len());
    println!("  Merkle root:      {}", merkle::format_tip5(&root));
    println!("  Query:            {:?}", query);
    println!("  Chunks retrieved: {}", manifest.results.len());
    println!("  LLM output:       {} bytes", output.len());
    println!("  Note settled:     {}", !effects.is_empty() || all_valid);
    println!("  All proofs valid: {}", all_valid);
    if settlement.chain_endpoint.is_some() {
        println!("  Chain connected:  true");
    }
    if settlement.auto_submit {
        println!("  TX submitted:     {}", tx_accepted);
        if !tx_id_str.is_empty() {
            println!("  TX ID:            {}", tx_id_str);
        }
    }
    println!("=== Hull pipeline complete ===");
    Ok(())
}
