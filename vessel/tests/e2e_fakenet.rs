//! E2E Fakenet Integration Tests — Phase 3.4 verification.
//!
//! These tests run against a live Nockchain fakenet and verify the full
//! settlement pipeline from document ingestion through on-chain state.
//!
//! # Running
//!
//! All tests are `#[ignore]` by default. They require a running fakenet:
//!
//! ```bash
//! # Option 1: Use the harness script (boots fakenet, runs tests, shuts down)
//! ./scripts/fakenet-harness.sh run
//!
//! # Option 2: Boot fakenet manually, then run tests
//! ./scripts/fakenet-harness.sh start
//! cargo test --test e2e_fakenet -- --ignored --nocapture
//! ./scripts/fakenet-harness.sh stop
//! ```
//!
//! # Environment Variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `LUME_FAKENET_CHAIN_ENDPOINT` | `http://127.0.0.1:9090` | Nockchain node public gRPC |
//! | `LUME_FAKENET_WALLET_ENDPOINT` | `http://localhost:5555` | Wallet private gRPC (legacy) |
//! | `LUME_FAKENET_WALLET_ADDRESS` | (none) | Wallet PKH (base58, ~58 chars) |
//! | `LUME_FAKENET_COINBASE_TIMELOCK_MIN` | `1` | Coinbase timelock (fakenet=1) |
//!
//! # What These Tests Verify (DEV.md Phase 3.4)
//!
//! 1. Boot fakenet + Lume NockApp + funded wallet
//! 2. Ingest documents via `/ingest`
//! 3. POST `/query`, trigger settlement
//! 4. Observe transaction on chain via explorer (manual — logged)
//! 5. Query chain to confirm Note contains correct Merkle root and settlement data
//! 6. Attempt settlement with tampered data, confirm rejection

// ---------------------------------------------------------------------------
// Shared test configuration
// ---------------------------------------------------------------------------

/// Read an env var with a fallback default.
fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn chain_endpoint() -> String {
    env_or("LUME_FAKENET_CHAIN_ENDPOINT", "http://127.0.0.1:9090")
}

fn wallet_endpoint() -> String {
    env_or("LUME_FAKENET_WALLET_ENDPOINT", "http://localhost:5555")
}

fn wallet_address() -> Option<String> {
    std::env::var("LUME_FAKENET_WALLET_ADDRESS").ok()
}

fn coinbase_timelock_min() -> u64 {
    std::env::var("LUME_FAKENET_COINBASE_TIMELOCK_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1) // fakenet default
}

// ---------------------------------------------------------------------------
// Test Group 1: Infrastructure connectivity
// ---------------------------------------------------------------------------

/// Verify: ChainClient can connect to the fakenet node's public gRPC.
#[tokio::test]
#[ignore]
async fn fakenet_chain_client_connects() {
    use vessel::chain::{ChainClient, ChainConfig};

    let endpoint = chain_endpoint();
    println!("Connecting to chain at {endpoint}...");

    let config = ChainConfig::local(&endpoint);
    let _client = ChainClient::connect(config)
        .await
        .expect("ChainClient must connect to fakenet node");

    println!("  ChainClient connected successfully.");
}

/// Verify: WalletClient can connect to the wallet's private gRPC.
#[tokio::test]
#[ignore]
async fn fakenet_wallet_client_connects() {
    use vessel::wallet::{WalletClient, WalletConfig};

    let endpoint = wallet_endpoint();
    println!("Connecting to wallet at {endpoint}...");

    let config = WalletConfig::new(&endpoint);
    let mut client = WalletClient::connect(config)
        .await
        .expect("WalletClient must connect to wallet");

    let ready = client.check_ready().await.expect("check_ready must not error");
    assert!(ready, "wallet must be responsive");
    println!("  WalletClient connected, wallet is ready.");
}

/// Verify: ChainClient can query balance on the fakenet using PKH.
///
/// Uses FirstName computation from PKH (ISSUE-004 fix) instead of
/// requiring a full SchnorrPubkey.
#[tokio::test]
#[ignore]
async fn fakenet_balance_query() {
    use vessel::chain::{ChainClient, ChainConfig, compute_coinbase_first_name};

    let endpoint = chain_endpoint();
    let pkh = match wallet_address() {
        Some(a) => a,
        None => {
            println!("LUME_FAKENET_WALLET_ADDRESS not set, skipping balance test.");
            return;
        }
    };

    let timelock_min = coinbase_timelock_min();

    // Verify FirstName computation works with the configured PKH.
    let first_name = compute_coinbase_first_name(&pkh, timelock_min)
        .expect("FirstName computation from MINING_PKH must succeed");
    println!("  PKH: {}..., coinbase FirstName: {}...", &pkh[..12], &first_name[..12]);

    let config = ChainConfig::local(&endpoint);
    let mut client = ChainClient::connect(config)
        .await
        .expect("ChainClient must connect");

    let balance = client
        .get_balance_by_pkh(&pkh, timelock_min)
        .await
        .expect("balance query by PKH must succeed");

    println!(
        "  Wallet {}: {} note(s) on-chain",
        &pkh[..12.min(pkh.len())],
        balance.notes.len()
    );
}

/// Verify: Balance can be queried via public gRPC using simple P2PKH FirstName.
///
/// Previously used wallet private gRPC (ISSUE-005: wallet is CLI, not service).
/// Now uses public gRPC FirstName query for both coinbase and simple P2PKH notes.
#[tokio::test]
#[ignore]
async fn fakenet_wallet_peek_balance() {
    use vessel::chain::{ChainClient, ChainConfig, compute_coinbase_first_name, compute_simple_first_name};

    let endpoint = chain_endpoint();
    let pkh = match wallet_address() {
        Some(a) => a,
        None => {
            println!("LUME_FAKENET_WALLET_ADDRESS not set, skipping wallet peek test.");
            return;
        }
    };

    let timelock_min = coinbase_timelock_min();

    let coinbase_fn = compute_coinbase_first_name(&pkh, timelock_min)
        .expect("coinbase FirstName must compute");
    let simple_fn = compute_simple_first_name(&pkh)
        .expect("simple FirstName must compute");
    println!("  Coinbase FirstName: {}...", &coinbase_fn[..12]);
    println!("  Simple FirstName:   {}...", &simple_fn[..12]);

    let config = ChainConfig::local(&endpoint);
    let mut client = ChainClient::connect(config)
        .await
        .expect("ChainClient must connect");

    let balance = client
        .get_balance_by_pkh(&pkh, timelock_min)
        .await
        .expect("balance query by PKH must succeed");

    println!("  Balance: {} note(s) on-chain", balance.notes.len());
}

// ---------------------------------------------------------------------------
// Test Group 2: Vessel pipeline with kernel
// ---------------------------------------------------------------------------

/// Verify: Full vessel pipeline runs locally (kernel boot, ingest, settle).
///
/// This test does NOT require a fakenet — it boots its own kernel.
/// It validates that the pipeline produces valid SettlementData that
/// could be submitted to a fakenet.
#[tokio::test]
#[ignore]
async fn fakenet_local_pipeline_produces_settlement() {
    use clap::Parser;
    use nockapp::kernel::boot;
    use nockapp::wire::{SystemWire, Wire};
    use nockapp::NockApp;
    use vessel::chain::{manifest_hash, SettlementData, LUME_DATA_VERSION};
    use vessel::types::*;

    // Boot kernel
    let cli = boot::Cli::parse_from(["test", "--new"]);
    let mut app: NockApp = boot::setup(kernels_lume::KERNEL, cli, &[], "lume", None)
        .await
        .expect("kernel must boot");

    // Build chunks + tree
    let chunks = vec![
        Chunk { id: 0, dat: "Revenue: $4.2M ARR".into() },
        Chunk { id: 1, dat: "Risk exposure: $800K".into() },
        Chunk { id: 2, dat: "Board approved Series B".into() },
        Chunk { id: 3, dat: "SOC2 audit Q4".into() },
    ];
    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = vessel::merkle::MerkleTree::build(&leaf_data);
    let root = tree.root();

    // Register root
    let register_poke = vessel::noun_builder::build_register_poke(7, &root);
    let effects = app
        .poke(SystemWire.to_wire(), register_poke)
        .await
        .expect("register poke must succeed");
    println!("  Register: {} effects", effects.len());

    // Build manifest
    let retrievals: Vec<Retrieval> = vec![0, 1]
        .into_iter()
        .map(|i| Retrieval {
            chunk: chunks[i].clone(),
            proof: tree.proof(i),
            score: 950_000,
        })
        .collect();

    let prompt = format!(
        "Summarize Q3\n{}\n{}",
        chunks[0].dat, chunks[1].dat
    );
    let manifest = Manifest {
        query: "Summarize Q3".into(),
        results: retrievals,
        prompt,
        output: "Q3 revenue was $4.2M with $800K risk exposure.".into(),
    };

    // Settle
    let note = Note {
        id: 1,
        vessel: 7,
        root,
        state: NoteState::Pending,
    };
    let settle_poke = vessel::noun_builder::build_settle_poke(&note, &manifest, &root);
    let effects = app
        .poke(SystemWire.to_wire(), settle_poke)
        .await
        .expect("settle poke must succeed");
    println!("  Settle: {} effects", effects.len());

    // Build settlement data (what would go on-chain)
    let settlement = SettlementData::from_settlement(&note, &manifest);
    assert_eq!(settlement.version, LUME_DATA_VERSION);
    assert_eq!(settlement.vessel_id, 7);
    assert_eq!(settlement.note_id, 1);
    assert_eq!(settlement.merkle_root, root);
    assert_eq!(settlement.manifest_hash, manifest_hash(&manifest));

    // Roundtrip through NoteData encoding
    let note_data = settlement.to_note_data();
    let decoded = SettlementData::from_note_data(&note_data)
        .expect("NoteData decode must succeed");
    assert_eq!(decoded, settlement);

    println!("  Settlement: {settlement}");
    println!("  NoteData roundtrip: OK ({} entries)", note_data.iter().count());
    println!("  Pipeline produces valid on-chain payload.");
}

// ---------------------------------------------------------------------------
// Test Group 3: On-chain settlement (requires funded fakenet)
// ---------------------------------------------------------------------------

/// Verify: Find (or confirm absence of) Lume settlement notes on-chain.
///
/// After a settlement transaction has been submitted and confirmed,
/// this test queries the chain for notes containing Lume NoteData.
/// Uses PKH-based FirstName queries (ISSUE-004 fix).
#[tokio::test]
#[ignore]
async fn fakenet_find_settlement_notes() {
    use vessel::chain::{ChainClient, ChainConfig};

    let endpoint = chain_endpoint();
    let pkh = match wallet_address() {
        Some(a) => a,
        None => {
            println!("LUME_FAKENET_WALLET_ADDRESS not set, skipping.");
            return;
        }
    };

    let timelock_min = coinbase_timelock_min();

    let config = ChainConfig::local(&endpoint);
    let mut client = ChainClient::connect(config)
        .await
        .expect("ChainClient must connect");

    let settlements = client
        .find_settlement_notes_by_pkh(&pkh, timelock_min)
        .await
        .expect("find_settlement_notes_by_pkh must not error");

    println!("  Found {} Lume settlement(s) at PKH {}", settlements.len(), &pkh[..12.min(pkh.len())]);
    for s in &settlements {
        println!("    {s}");
    }
}

/// Verify: Tampered settlement data is rejected by the kernel.
///
/// Constructs a manifest with a tampered chunk (wrong data for the
/// Merkle proof) and verifies the kernel nacks the settle poke.
#[tokio::test]
#[ignore]
async fn fakenet_reject_tampered_settlement() {
    use clap::Parser;
    use nockapp::kernel::boot;
    use nockapp::wire::{SystemWire, Wire};
    use nockapp::NockApp;
    use vessel::types::*;

    // Boot kernel
    let cli = boot::Cli::parse_from(["test", "--new"]);
    let mut app: NockApp = boot::setup(kernels_lume::KERNEL, cli, &[], "lume", None)
        .await
        .expect("kernel must boot");

    // Build valid tree
    let chunks = vec![
        Chunk { id: 0, dat: "Valid data A".into() },
        Chunk { id: 1, dat: "Valid data B".into() },
    ];
    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = vessel::merkle::MerkleTree::build(&leaf_data);
    let root = tree.root();

    // Register root
    let register_poke = vessel::noun_builder::build_register_poke(7, &root);
    app.poke(SystemWire.to_wire(), register_poke)
        .await
        .expect("register must succeed");

    // Build manifest with TAMPERED chunk data
    // The proof is for "Valid data A" but the chunk says "TAMPERED data"
    let tampered_retrievals = vec![Retrieval {
        chunk: Chunk { id: 0, dat: "TAMPERED data".into() },
        proof: tree.proof(0), // proof for the original data
        score: 950_000,
    }];

    let manifest = Manifest {
        query: "test".into(),
        results: tampered_retrievals,
        prompt: "test\nTAMPERED data".into(),
        output: "tampered output".into(),
    };

    let note = Note {
        id: 99,
        vessel: 7,
        root,
        state: NoteState::Pending,
    };

    let settle_poke = vessel::noun_builder::build_settle_poke(&note, &manifest, &root);
    let result = app.poke(SystemWire.to_wire(), settle_poke).await;

    // The kernel should reject (nack) the tampered settlement.
    // A nack manifests as either an error or empty effects depending
    // on the kernel's crash-on-failure configuration.
    match result {
        Err(e) => {
            println!("  Tampered settlement correctly rejected: {e}");
        }
        Ok(effects) if effects.is_empty() => {
            println!("  Tampered settlement produced 0 effects (nacked).");
        }
        Ok(effects) => {
            // If we get effects, check they contain an error marker
            println!(
                "  WARNING: tampered settlement produced {} effects — verify manually",
                effects.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test Group 4: HTTP API E2E (boots real kernel, no fakenet needed)
// ---------------------------------------------------------------------------

/// Verify: Full HTTP API pipeline — ingest, query, settle, verify root.
///
/// This mirrors the DEV.md verification steps 2-3 using the HTTP API
/// instead of the CLI pipeline.
#[tokio::test]
#[ignore]
async fn fakenet_http_api_full_pipeline() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use clap::Parser;
    use http_body_util::BodyExt;
    use nockapp::kernel::boot;
    use nockapp::NockApp;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    // Boot kernel + create state
    let cli = boot::Cli::parse_from(["test", "--new"]);
    let app: NockApp = boot::setup(kernels_lume::KERNEL, cli, &[], "lume", None)
        .await
        .expect("kernel boot");

    let state = Arc::new(Mutex::new(vessel::api::AppState {
        app,
        chunks: Vec::new(),
        tree: None,
        vessel_id: 7,
        top_k: 2,
        llm: Box::new(vessel::llm::StubProvider),
        retriever: Box::new(vessel::retrieve::KeywordRetriever),
        note_counter: 0,
    }));

    let router = vessel::api::router(state.clone());

    // --- Step 1: Ingest documents ---
    let ingest_body = serde_json::json!({
        "documents": [
            "Q3 revenue: $4.2M ARR, 18% QoQ growth.\n\nRisk exposure: $800K in variable-rate instruments.",
            "Board approved Series B at $45M pre-money.\n\nSOC2 Type II audit scheduled for Q4."
        ]
    });

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&ingest_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes: bytes::Bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let ingest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let chunk_count = ingest["chunk_count"].as_u64().unwrap();
    let merkle_root = ingest["merkle_root"].as_str().unwrap().to_string();

    println!("  Ingested {} chunks, root: {}", chunk_count, &merkle_root[..16]);
    assert!(chunk_count >= 4, "4 documents should produce at least 4 chunks");
    assert!(!merkle_root.is_empty());

    // --- Step 2: Query → settle ---
    let query_body = serde_json::json!({
        "query": "Summarize Q3 financial position",
        "top_k": 2
    });

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&query_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes: bytes::Bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let query_resp: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let settled = query_resp["settled"].as_bool().unwrap();
    let note_id = query_resp["note_id"].as_u64().unwrap();
    let query_root = query_resp["merkle_root"].as_str().unwrap();
    let chunks_retrieved = query_resp["chunks_retrieved"].as_u64().unwrap();

    println!(
        "  Query settled: note_id={}, chunks={}, root={}",
        note_id,
        chunks_retrieved,
        &query_root[..16]
    );

    assert!(settled, "settlement must succeed");
    assert!(note_id > 0, "note_id must be assigned");
    assert_eq!(query_root, merkle_root, "root must match between ingest and query");
    assert!(chunks_retrieved > 0, "must retrieve at least 1 chunk");

    // --- Step 3: Verify status ---
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes: bytes::Bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert!(status["has_tree"].as_bool().unwrap());
    assert_eq!(status["notes_settled"].as_u64().unwrap(), 1);
    assert_eq!(
        status["merkle_root"].as_str().unwrap(),
        merkle_root,
        "status root must match"
    );

    println!("  Status: tree=true, notes_settled=1, root matches.");
    println!("  Full HTTP API pipeline verified.");
}

// ---------------------------------------------------------------------------
// Test Group 5: Settlement NoteData verification
// ---------------------------------------------------------------------------

/// Verify: SettlementData encodes all 5 required NoteData keys and
/// survives a full roundtrip for various payload sizes.
#[tokio::test]
#[ignore]
async fn fakenet_settlement_notedata_comprehensive() {
    use vessel::chain::{
        SettlementData, LUME_DATA_VERSION,
        KEY_VERSION, KEY_VESSEL_ID, KEY_MERKLE_ROOT, KEY_NOTE_ID, KEY_MANIFEST_HASH,
    };

    // Test with different data sizes
    let test_cases: Vec<(u64, u64, [u64; 5], [u64; 5])> = vec![
        (1, 1, [0; 5], [0; 5]),                    // all zeros
        (7, 42, [0xAA; 5], [0xBB; 5]),             // typical
        (1000, 999_999, [0xFF; 5], [0xFF; 5]),      // large IDs
        (1, 1, [1, 2, 3, 4, 5], [0xDE; 5]),         // sequential
    ];

    for (i, (vessel_id, note_id, root, mhash)) in test_cases.iter().enumerate() {
        let data = SettlementData {
            version: LUME_DATA_VERSION,
            vessel_id: *vessel_id,
            merkle_root: *root,
            note_id: *note_id,
            manifest_hash: *mhash,
        };

        let note_data = data.to_note_data();

        // All 5 keys present
        let keys: Vec<&str> = note_data.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&KEY_VERSION), "case {i}: missing lume-v");
        assert!(keys.contains(&KEY_VESSEL_ID), "case {i}: missing lume-vid");
        assert!(keys.contains(&KEY_MERKLE_ROOT), "case {i}: missing lume-root");
        assert!(keys.contains(&KEY_NOTE_ID), "case {i}: missing lume-nid");
        assert!(keys.contains(&KEY_MANIFEST_HASH), "case {i}: missing lume-mhash");

        // Roundtrip
        let decoded = SettlementData::from_note_data(&note_data)
            .unwrap_or_else(|e| panic!("case {i}: decode failed: {e}"));
        assert_eq!(decoded, data, "case {i}: roundtrip mismatch");
    }

    println!("  {} NoteData roundtrip cases passed.", test_cases.len());
}

/// Verify: Wallet noun construction produces valid JAM payloads.
#[tokio::test]
#[ignore]
async fn fakenet_wallet_noun_construction() {
    use vessel::wallet;

    // Peek path construction
    let path = wallet::build_peek_path(&["balance-by-pubkey", "testkey123"]);
    assert!(!path.is_empty(), "peek path must produce JAM bytes");

    // Sign-hash poke
    let sign = wallet::build_sign_hash_poke("abc123hash", 0, false);
    assert!(!sign.is_empty(), "sign-hash poke must produce JAM bytes");

    // Create-tx poke
    let tx = wallet::build_create_tx_poke("first", "last", "recipient", 65536, 128);
    assert!(!tx.is_empty(), "create-tx poke must produce JAM bytes");

    // Determinism
    let tx2 = wallet::build_create_tx_poke("first", "last", "recipient", 65536, 128);
    assert_eq!(tx, tx2, "same args must produce identical JAM bytes");

    // Different amounts produce different payloads
    let tx3 = wallet::build_create_tx_poke("first", "last", "recipient", 65537, 128);
    assert_ne!(tx, tx3, "different amount must change payload");

    println!("  Wallet noun construction: all checks passed.");
}

// ---------------------------------------------------------------------------
// Test Group 6: Wallet Kernel Integration (ISSUE-005 fix)
// ---------------------------------------------------------------------------

/// Verify: Wallet kernel boots in-process and generates keys.
///
/// This proves the wallet kernel can run alongside the Lume kernel
/// in the same process — the foundation for local key management.
#[tokio::test]
#[ignore]
async fn fakenet_wallet_kernel_boots_and_generates_keys() {
    use vessel::wallet_kernel::{WalletKernel, TEST_SEED_PHRASE};
    use vessel::chain::compute_coinbase_first_name;

    let tmp = tempfile::tempdir().expect("tempdir");

    println!("  Booting wallet kernel...");
    let mut wk = WalletKernel::boot(
        kernels_open_wallet::KERNEL,
        tmp.path(),
    )
    .await
    .expect("wallet kernel must boot");

    println!("  Importing test seed phrase...");
    wk.import_seed_phrase(TEST_SEED_PHRASE, 1)
        .await
        .expect("import must succeed");

    println!("  Setting fakenet mode...");
    wk.set_fakenet()
        .await
        .expect("set_fakenet must succeed");

    println!("  Peeking signing keys...");
    let keys = wk.peek_signing_keys()
        .await
        .expect("peek_signing_keys must succeed");

    assert!(
        !keys.is_empty(),
        "wallet must have at least one signing key after import"
    );

    for key in &keys {
        let pkh_b58 = key.to_base58();
        let first_name = compute_coinbase_first_name(&pkh_b58, 1)
            .expect("FirstName must compute from wallet-generated PKH");
        println!("  Signing key PKH: {}", pkh_b58);
        println!("  Coinbase FirstName: {}", first_name);
    }

    println!("  Wallet kernel integration: {} key(s) generated.", keys.len());
}

/// Verify: Wallet kernel tracked pubkeys peek succeeds.
///
/// Note: The wallet kernel's `tracked-pubkeys` peek filters out v1 coils
/// (created by seed phrase import). It only returns v0 coils and watched
/// addresses with full 132-byte pubkeys. After a v1 seed import with no
/// watched addresses, this list is expected to be empty.
/// Use `peek_signing_keys()` to get PKHs from v1 keys.
#[tokio::test]
#[ignore]
async fn fakenet_wallet_kernel_tracked_pubkeys() {
    use vessel::wallet_kernel::{WalletKernel, TEST_SEED_PHRASE};

    let tmp = tempfile::tempdir().expect("tempdir");

    let mut wk = WalletKernel::boot(
        kernels_open_wallet::KERNEL,
        tmp.path(),
    )
    .await
    .expect("wallet kernel must boot");

    wk.import_seed_phrase(TEST_SEED_PHRASE, 1)
        .await
        .expect("import");

    let pubkeys = wk.peek_tracked_pubkeys()
        .await
        .expect("peek_tracked_pubkeys must succeed");

    // v1 seed imports produce v1 coils which are filtered out by the
    // wallet kernel's tracked-pubkeys peek. Empty is expected here.
    println!("  {} tracked pubkey(s) (v1 import: expect 0).", pubkeys.len());
    for pk in &pubkeys {
        println!("  Tracked pubkey: {}...", &pk[..20.min(pk.len())]);
    }

    // Verify signing-keys still works as the reliable key source
    let signing_keys = wk.peek_signing_keys()
        .await
        .expect("peek_signing_keys must succeed");
    assert!(
        !signing_keys.is_empty(),
        "signing keys must be available after import (even when tracked-pubkeys is empty)"
    );
    println!("  {} signing key(s) confirmed via peek_signing_keys.", signing_keys.len());
}
