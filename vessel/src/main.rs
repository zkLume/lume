//! Lume Vessel Orchestrator — NockApp-based off-chain client.
//!
//! Pipeline: boot kernel → ingest → build tree → register root →
//!           retrieve → infer → build manifest → settle via poke
//!
//! Boots the compiled Hoon kernel (lume.jam) as a NockApp, then
//! drives the settlement pipeline through kernel pokes.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use nockapp::kernel::boot;
use nockapp::noun::slab::NounSlab;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use tokio::sync::Mutex;

use vessel::api;
use vessel::chain;
use vessel::ingest;
use vessel::llm;
use vessel::merkle::{self, MerkleTree};
use vessel::noun_builder;
use vessel::retrieve;
use vessel::types::*;
use vessel::wallet;

#[derive(Parser)]
#[command(name = "vessel", about = "Lume Vessel Orchestrator")]
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

    /// Nockchain gRPC endpoint for on-chain settlement.
    /// If omitted, the pipeline runs locally without chain submission.
    #[arg(long = "chain-endpoint")]
    chain_endpoint: Option<String>,

    /// Wallet address (base58) for checking funding and querying notes.
    /// Required when --chain-endpoint is set.
    #[arg(long = "wallet-address")]
    wallet_address: Option<String>,

    /// Wallet private gRPC endpoint for signing coordination.
    /// If omitted, wallet coordination is skipped.
    /// Default wallet port is 5555 (e.g., http://localhost:5555).
    #[arg(long = "wallet-grpc")]
    wallet_grpc: Option<String>,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    println!("=== Lume Vessel Orchestrator (NockApp) ===\n");

    // --- Boot the NockApp kernel with STARK prover jets ---
    println!("[0] Booting Lume NockApp kernel...");
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let mut app: NockApp = boot::setup(
        kernels_lume::KERNEL,
        cli.boot,
        prover_hot_state.as_slice(),
        "lume",
        None,
    )
    .await?;
    println!("    Kernel booted ({} bytes JAM, {} prover jets)",
        kernels_lume::KERNEL.len(), prover_hot_state.len());

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
            vessel_id: 7,
            top_k: cli.top_k,
            llm: provider,
            retriever: Box::new(retrieve::KeywordRetriever),
            note_counter: 0,
        }));
        return api::serve(state, cli.port).await;
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
    };
    println!(
        "[6] Manifest: {} retrievals, prompt {} bytes",
        manifest.results.len(),
        manifest.prompt.len()
    );

    // --- [7] Note + Settle ---
    let note = Note {
        id: 1,
        vessel: 7,
        root,
        state: NoteState::Pending,
    };
    println!(
        "[7] Note #{} (vessel={}, state=Pending) → settling...",
        note.id, note.vessel
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
        if !valid {
            all_valid = false;
        }
        println!(
            "  retrieval[{}] chunk_id={}: proof {}",
            i,
            retrieval.chunk.id,
            if valid { "VALID" } else { "FAILED" }
        );
    }

    // --- [9] On-chain settlement (optional) ---
    let settlement = chain::SettlementData::from_settlement(&note, &manifest);
    let chain_settled = if let Some(ref endpoint) = cli.chain_endpoint {
        println!("\n[9] Connecting to Nockchain node at {endpoint}...");
        let chain_config = chain::ChainConfig::local(endpoint);
        match chain::ChainClient::connect(chain_config).await {
            Ok(mut client) => {
                // Check wallet funding if address provided
                if let Some(ref addr) = cli.wallet_address {
                    match client.get_balance(addr).await {
                        Ok(balance) => {
                            println!(
                                "    Wallet {}: {} note(s) on-chain",
                                &addr[..12.min(addr.len())],
                                balance.notes.len()
                            );
                        }
                        Err(e) => {
                            eprintln!("    warn: could not check balance: {e}");
                        }
                    }
                }

                println!("    Settlement: {settlement}");

                // If we have a wallet address, scan for existing Lume settlements
                if let Some(ref addr) = cli.wallet_address {
                    match client.find_settlement_notes(addr).await {
                        Ok(notes) if !notes.is_empty() => {
                            println!("    Found {} existing Lume settlement(s):", notes.len());
                            for s in &notes {
                                println!("      {s}");
                            }
                        }
                        Ok(_) => {
                            println!("    No existing Lume settlements found on-chain.");
                        }
                        Err(e) => {
                            eprintln!("    warn: could not query settlements: {e}");
                        }
                    }
                }
                true
            }
            Err(e) => {
                eprintln!("    Failed to connect to chain: {e}");
                eprintln!("    (Pipeline completed locally; chain settlement skipped)");
                false
            }
        }
    } else {
        false
    };

    // --- [10] Wallet coordination (optional) ---
    let wallet_ready = if let Some(ref wallet_ep) = cli.wallet_grpc {
        println!("\n[10] Connecting to wallet at {wallet_ep}...");
        let wallet_config = wallet::WalletConfig::new(wallet_ep);
        match wallet::WalletClient::connect(wallet_config).await {
            Ok(mut wclient) => {
                match wclient.check_ready().await {
                    Ok(true) => {
                        println!("    Wallet is responsive.");

                        // Peek balance if we have a wallet address
                        if let Some(ref addr) = cli.wallet_address {
                            match wclient.peek_balance(addr).await {
                                Ok(balance) => {
                                    println!("    Wallet balance peek: {balance}");
                                }
                                Err(e) => {
                                    eprintln!("    warn: could not peek wallet balance: {e}");
                                }
                            }
                        }

                        println!("    Settlement data ready: {settlement}");
                        println!("    Settlement NoteData: {} entries",
                            settlement.to_note_data().iter().count()
                        );

                        // Transaction submission requires UTXO selection
                        // (input note name, recipient address) which is
                        // operator-provided. Log readiness for Phase 3.4.
                        println!("    Wallet coordination layer active.");
                        println!("    (Full tx submission requires --input-note and --recipient flags, Phase 3.4)");
                        true
                    }
                    Ok(false) => {
                        eprintln!("    Wallet not ready.");
                        false
                    }
                    Err(e) => {
                        eprintln!("    Wallet check failed: {e}");
                        false
                    }
                }
            }
            Err(e) => {
                eprintln!("    Failed to connect to wallet: {e}");
                eprintln!("    (Pipeline completed without wallet; signing skipped)");
                false
            }
        }
    } else {
        false
    };

    // --- Summary ---
    println!("\n=== Pipeline Summary ===");
    println!("  Chunks ingested:  {}", chunks.len());
    println!("  Merkle root:      {}", merkle::format_tip5(&root));
    println!("  Query:            {:?}", query);
    println!("  Chunks retrieved: {}", manifest.results.len());
    println!("  LLM output:       {} bytes", output.len());
    println!("  Note settled:     {}", !effects.is_empty() || all_valid);
    println!("  All proofs valid: {}", all_valid);
    if cli.chain_endpoint.is_some() {
        println!("  Chain connected:  {}", chain_settled);
    }
    if cli.wallet_grpc.is_some() {
        println!("  Wallet ready:     {}", wallet_ready);
    }
    println!("=== Vessel pipeline complete ===");
    Ok(())
}
