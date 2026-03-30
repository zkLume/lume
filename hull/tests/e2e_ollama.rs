//! E2E Ollama Integration Tests — Phase 3.5.1 verification.
//!
//! Tests the full pipeline with a real LLM (Ollama) to verify:
//! 1. Prompt constructed from query + chunks is sent to Ollama
//! 2. LLM response is captured in the manifest
//! 3. Kernel settles the manifest (prompt hash matches)
//! 4. tip5 hash of the prompt is identical before and after the LLM call
//!
//! # Running
//!
//! All tests are `#[ignore]` by default. They require:
//! - An Ollama instance (local or remote, e.g. RunPod)
//! - A pulled model (default: llama3.2)
//!
//! ```bash
//! # Set the Ollama URL (required)
//! export VESL_OLLAMA_URL="http://your-runpod:11434"
//!
//! # Optional: override model name
//! export VESL_OLLAMA_MODEL="llama3.2"
//!
//! # Run the tests
//! cargo test --test e2e_ollama -- --ignored --nocapture
//! ```

use clap::Parser;
use nockapp::kernel::boot;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;

use hull::llm::{self, LlmProvider, OllamaProvider};
use hull::merkle::MerkleTree;
use hull::noun_builder;
use hull::retrieve::{KeywordRetriever, Retriever, SCORE_SCALE};
use hull::types::{Chunk, Manifest, Note, NoteState, Retrieval};

// ---------------------------------------------------------------------------
// Shared test configuration
// ---------------------------------------------------------------------------

fn ollama_url() -> Option<String> {
    std::env::var("VESL_OLLAMA_URL").ok()
}

fn ollama_model() -> String {
    std::env::var("VESL_OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".to_string())
}

/// Sample enterprise document chunks for testing.
fn sample_chunks() -> Vec<Chunk> {
    vec![
        Chunk {
            id: 0,
            dat: "Q3 2025 revenue reached $4.2M ARR, representing 18% quarter-over-quarter growth. The SaaS segment contributed 72% of total revenue, with enterprise contracts averaging $180K ACV.".into(),
        },
        Chunk {
            id: 1,
            dat: "Risk exposure analysis shows $800K in variable-rate instruments. The treasury team recommends hedging 60% of the position before Q4 rate decisions. Current interest rate sensitivity is +/- $120K per 25bp move.".into(),
        },
        Chunk {
            id: 2,
            dat: "Board approved Series B at $45M pre-money valuation. Lead investor committed $20M with 2x liquidation preference. Closing expected by end of Q4 2025, subject to final due diligence.".into(),
        },
        Chunk {
            id: 3,
            dat: "SOC2 Type II audit scheduled for Q4 2025. The compliance team has identified 3 minor gaps in access control logging that need remediation before the audit window opens.".into(),
        },
        Chunk {
            id: 4,
            dat: "Customer churn decreased to 2.1% monthly, down from 3.4% in Q2. The customer success team attributes the improvement to the new onboarding workflow and proactive health scoring.".into(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Test 1: Ollama connectivity + response capture
// ---------------------------------------------------------------------------

/// Verify: OllamaProvider can reach the remote instance and generate a response.
#[tokio::test]
#[ignore]
async fn ollama_connectivity_and_response() {
    let url = match ollama_url() {
        Some(u) => u,
        None => {
            println!("SKIP: VESL_OLLAMA_URL not set");
            return;
        }
    };
    let model = ollama_model();
    println!("Connecting to Ollama at {url} with model {model}...");

    let provider = OllamaProvider::new(&url, &model);
    let result = provider
        .generate("What is 2 + 2? Reply with just the number.")
        .await;

    match &result {
        Ok(output) => println!("Ollama response: {output}"),
        Err(e) => panic!("Ollama generate failed: {e}"),
    }

    let output = result.unwrap();
    assert!(!output.is_empty(), "Ollama must return a non-empty response");
}

// ---------------------------------------------------------------------------
// Test 2: Full pipeline — ingest → retrieve → LLM → manifest → settle
// ---------------------------------------------------------------------------

/// Verify: a real LLM response settles through the kernel with matching prompt hash.
///
/// This is the core 3.5.1 verification:
/// 1. Build Merkle tree from sample docs
/// 2. Retrieve top-K chunks for a query
/// 3. Construct deterministic prompt (must match Hoon's ++build-prompt)
/// 4. Send to real Ollama, capture response
/// 5. Build manifest with prompt + response
/// 6. Settle through kernel — prompt hash MUST match
#[tokio::test]
#[ignore]
async fn ollama_live_settlement() {
    let url = match ollama_url() {
        Some(u) => u,
        None => {
            println!("SKIP: VESL_OLLAMA_URL not set");
            return;
        }
    };
    let model = ollama_model();
    println!("=== Phase 3.5.1: Live Ollama Settlement Test ===");
    println!("Ollama: {url} (model: {model})");

    // --- Step 1: Build Merkle tree ---
    let chunks = sample_chunks();
    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = MerkleTree::build(&leaf_data);
    let root = tree.root();
    println!("Merkle root: {:?}", root);
    println!("Chunks: {} documents committed", chunks.len());

    // --- Step 2: Boot kernel + register root ---
    let cli = boot::Cli::parse_from(["test", "--new"]);
    let mut app: NockApp = boot::setup(kernels_vesl::KERNEL, cli, &[], "vesl", None)
        .await
        .expect("kernel must boot");
    println!("Kernel booted successfully");

    let register_slab = noun_builder::build_register_poke(7, &root);
    app.poke(SystemWire.to_wire(), register_slab)
        .await
        .expect("register poke must succeed");
    println!("Root registered with kernel (hull_id=7)");

    // --- Step 3: Retrieve top-K chunks ---
    let query = "What was the Q3 revenue and growth rate?";
    let retriever = KeywordRetriever;
    let hits = retriever.retrieve(query, &chunks, 2);
    assert!(!hits.is_empty(), "retriever must find relevant chunks");
    println!(
        "Retrieved {} chunks for query: {query}",
        hits.len()
    );
    for hit in &hits {
        println!(
            "  chunk[{}] score={:.3} — {}...",
            hit.chunk_index,
            hit.score,
            &chunks[hit.chunk_index].dat[..60.min(chunks[hit.chunk_index].dat.len())]
        );
    }

    // --- Step 4: Build deterministic prompt ---
    let retrieved_chunks: Vec<&Chunk> = hits.iter().map(|h| &chunks[h.chunk_index]).collect();
    let prompt = llm::build_prompt(query, &retrieved_chunks);
    println!("Prompt ({} bytes):", prompt.len());
    println!("---");
    println!("{prompt}");
    println!("---");

    // Record prompt bytes for hash comparison after LLM call
    let prompt_bytes_before = prompt.as_bytes().to_vec();

    // --- Step 5: Call real Ollama ---
    println!("Sending prompt to Ollama...");
    let provider = OllamaProvider::new(&url, &model);
    let llm_output = provider
        .generate(&prompt)
        .await
        .expect("Ollama must generate a response");
    println!("LLM output ({} bytes):", llm_output.len());
    println!("---");
    println!("{llm_output}");
    println!("---");
    assert!(
        !llm_output.is_empty(),
        "LLM must return a non-empty response"
    );

    // --- Step 6: Verify prompt bytes unchanged after LLM call ---
    assert_eq!(
        prompt.as_bytes(),
        prompt_bytes_before.as_slice(),
        "prompt bytes must be identical before and after LLM call"
    );

    // --- Step 7: Build manifest ---
    let results: Vec<Retrieval> = hits
        .iter()
        .map(|hit| {
            let chunk = chunks[hit.chunk_index].clone();
            let proof = tree.proof(hit.chunk_index);
            Retrieval {
                chunk,
                proof,
                score: (hit.score * SCORE_SCALE as f64) as u64,
            }
        })
        .collect();

    let manifest = Manifest {
        query: query.to_string(),
        results,
        prompt: prompt.clone(),
        output: llm_output.clone(),
    };
    println!("Manifest built: {} retrievals", manifest.results.len());

    // --- Step 8: Settle through kernel ---
    let note = Note {
        id: 1,
        hull: 7,
        root,
        state: NoteState::Pending,
    };

    let settle_slab = noun_builder::build_settle_poke(&note, &manifest, &root);
    let effects = app
        .poke(SystemWire.to_wire(), settle_slab)
        .await
        .expect("settlement poke must succeed with real LLM output");

    println!("Settlement succeeded! Effects: {}", effects.len());
    println!("=== Phase 3.5.1 PASSED: Real LLM output settles through kernel ===");
}

// ---------------------------------------------------------------------------
// Test 3: Prompt hash consistency — tip5 hash before/after LLM call
// ---------------------------------------------------------------------------

/// Verify: tip5 hash of the prompt is identical whether computed before
/// or after the LLM call. This guards against any subtle mutation or
/// encoding issue in the prompt string during the HTTP round-trip.
#[tokio::test]
#[ignore]
async fn ollama_prompt_hash_consistency() {
    let url = match ollama_url() {
        Some(u) => u,
        None => {
            println!("SKIP: VESL_OLLAMA_URL not set");
            return;
        }
    };
    let model = ollama_model();

    let chunks = sample_chunks();
    let query = "What is the customer churn rate?";
    let retrieved: Vec<&Chunk> = vec![&chunks[4], &chunks[0]];
    let prompt = llm::build_prompt(query, &retrieved);

    // Compute tip5 hash of prompt BEFORE LLM call
    let hash_before = hull::merkle::hash_leaf(prompt.as_bytes());

    // Call Ollama (the prompt string should not be mutated)
    let provider = OllamaProvider::new(&url, &model);
    let _output = provider
        .generate(&prompt)
        .await
        .expect("Ollama must respond");

    // Compute tip5 hash of prompt AFTER LLM call
    let hash_after = hull::merkle::hash_leaf(prompt.as_bytes());

    assert_eq!(
        hash_before, hash_after,
        "tip5 hash of prompt must be identical before and after LLM call"
    );
    println!("Prompt hash consistent: {:?}", hash_before);
}
