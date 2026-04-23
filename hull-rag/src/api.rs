//! HTTP API driver — axum server wrapping the hull pipeline.
//!
//! Phase 2.4 of the DEV.md roadmap. Provides REST endpoints that drive
//! the full ingest → retrieve → LLM → settle pipeline.
//!
//! # Architecture
//!
//! The HTTP layer lives in Rust (not Hoon) because the pipeline's heavy
//! lifting — ingestion, retrieval, LLM inference — is all Rust-side.
//! The Hoon kernel is poked only for settlement (verify manifest + state
//! transition). This matches the NockApp pattern: HTTP requests become
//! kernel pokes; effects become HTTP responses.
//!
//! Shared state is held behind `Arc<Mutex<AppState>>` so axum handlers
//! can access the kernel, chunks, and tree concurrently.
//!
//! # Endpoints
//!
//! | Method | Path      | Function                                          |
//! |--------|-----------|---------------------------------------------------|
//! | POST   | `/ingest` | Upload text, chunk it, build tree, register root  |
//! | POST   | `/query`  | Retrieve chunks, call LLM, settle via kernel poke |
//! | GET    | `/status` | Return current state (root, chunk count, notes)   |
//! | GET    | `/health` | Liveness check                                    |

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use nockapp::kernel::boot::NockStackSize;
use nockapp::noun::slab::{NockJammer, NounSlab};
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use nock_noun_rs::slab_root;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::limit::RequestBodyLimitLayer;

use crate::chain;
use crate::config::SettlementConfig;
use crate::llm::{self, LlmProvider};
use crate::merkle::{hash_leaf, MerkleTree};
use crate::noun_builder;
use crate::retrieve::Retriever;
use crate::signing;
use crate::tx_builder;
use crate::types::*;

/// Derive a note ID from query + timestamp + random nonce (H-005).
///
/// Prevents cross-instance replay: two hull instances processing the same
/// documents can't produce colliding note/hull/root tuples.
///
/// AUDIT 2026-04-17 H-05: fails closed on `getrandom::fill` error. The
/// prior silent fallback (copy timestamp bytes into nonce) reduced the
/// note-id's entropy to `H(query || timestamp)` — both values are
/// attacker-observable over an open network. Combined with H-03's
/// pre-commit race, that was a zero-secret DoS. Panic is correct here:
/// a production host without `/dev/urandom` is misconfigured and
/// should not keep serving settlement requests.
fn derive_note_id(query: &str) -> u64 {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .expect("getrandom::fill failed — OS entropy unavailable (refusing to derive note-id from attacker-observable state)");
    let mut content = Vec::with_capacity(query.len() + 16 + 16);
    content.extend_from_slice(query.as_bytes());
    content.extend_from_slice(&(timestamp as u64).to_le_bytes());
    content.extend_from_slice(&nonce);
    let digest = hash_leaf(&content);
    // Truncate tip5 digest to u64 (first limb). Ensure non-zero.
    //
    // AUDIT 2026-04-17 L-02: collision analysis. With the graft's
    // 1M-per-epoch cap (settle-graft H-01), the total simultaneously
    // live note-id space is ~2M (current + prior epoch). Birthday
    // bound on a 64-bit keyspace is ~2^32; 2^21 items give a
    // collision probability of roughly N^2 / 2k = 2^42 / 2^65 = 2^-23,
    // about 1 in 8M over a full graft lifetime. Acceptable.
    // Promote to a full tip5 digest if/when higher throughput makes
    // this uncomfortable.
    let id = digest[0];
    if id == 0 { digest[1] | 1 } else { id }
}

/// Return at most `chars` characters from the start of `s`, splitting
/// on a UTF-8 character boundary.
///
/// AUDIT 2026-04-19 H-09: byte-range slicing (`&s[..n]`) panics when `n`
/// falls mid-codepoint. Every handler preview path touched user input
/// where that was reachable — a query of `"中".repeat(40)` crashes the
/// task. Use this helper anywhere a preview truncation length is in
/// characters, not bytes.
pub fn char_safe_prefix(s: &str, chars: usize) -> &str {
    let end = s
        .char_indices()
        .nth(chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[..end]
}

/// Hash a retrieval set for TOCTOU detection (C-003).
///
/// Hashes chunk IDs + data bytes into a single tip5 digest.
/// Compared between phase 1 (build) and phase 3 (settle) to detect
/// mutations that occurred while the lock was released for LLM inference.
fn hash_retrievals(retrievals: &[Retrieval]) -> Tip5Hash {
    let mut content = Vec::new();
    for r in retrievals {
        content.extend_from_slice(&r.chunk.id.to_le_bytes());
        content.extend_from_slice(r.chunk.dat.as_bytes());
    }
    hash_leaf(&content)
}

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Shared state for the HTTP API.
///
/// Held behind `Arc<Mutex<...>>` so axum handlers can access it.
/// The Mutex is tokio-aware so `.lock().await` doesn't block the runtime.
pub struct AppState {
    pub app: NockApp,
    pub chunks: Vec<Chunk>,
    pub tree: Option<MerkleTree>,
    pub hull_id: u64,
    pub top_k: usize,
    pub retriever: Box<dyn Retriever + Send + Sync>,
    /// Count of settled notes (incremented per successful /query).
    pub note_counter: u64,
    /// Settlement configuration (mode + chain settings).
    pub settlement: SettlementConfig,
    /// Nock stack size the kernel was booted with.
    pub stack_size: NockStackSize,
    /// Output directory for persistence files (note_counter, etc.).
    pub output_dir: PathBuf,
    /// Ring buffer of recently settled notes (last N summaries).
    pub recent_notes: std::collections::VecDeque<NoteSummary>,
}

/// Summary of a settled note, kept in a ring buffer for /status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSummary {
    pub note_id: u64,
    pub root: String,
    pub query_preview: String,
    pub settled: bool,
}

const MAX_RECENT_NOTES: usize = 20;

/// Wrapper that holds the mutex-protected state and the LLM provider separately.
/// LLM inference can take 30+ seconds; keeping it outside the mutex prevents
/// blocking /health, /status, and other handlers during inference (V-003b).
pub struct ServerState {
    pub inner: Mutex<AppState>,
    pub llm: Box<dyn LlmProvider>,
}

pub type SharedState = Arc<ServerState>;

// ---------------------------------------------------------------------------
// Note counter persistence
// ---------------------------------------------------------------------------

const NOTE_COUNTER_FILE: &str = ".hull_note_counter";

pub fn load_note_counter(output_dir: &std::path::Path) -> u64 {
    let path = output_dir.join(NOTE_COUNTER_FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn save_note_counter(output_dir: &std::path::Path, counter: u64) {
    // AUDIT 2026-04-17 L-05: atomic write via tempfile + rename to
    // avoid torn writes. Single-writer invariant (one hull per
    // output_dir) is still the caller's responsibility.
    let path = output_dir.join(NOTE_COUNTER_FILE);
    let tmp = output_dir.join(format!("{NOTE_COUNTER_FILE}.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, counter.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct IngestRequest {
    /// Raw text documents to ingest. Each string becomes one "file".
    pub documents: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct IngestResponse {
    pub chunk_count: usize,
    pub merkle_root: String,
    pub status: String,
}

#[derive(Deserialize)]
pub struct QueryRequest {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: Option<usize>,
}

fn default_top_k() -> Option<usize> {
    None
}

#[derive(Serialize, Deserialize)]
pub struct QueryResponse {
    pub query: String,
    pub chunks_retrieved: usize,
    pub retrievals: Vec<RetrievalInfo>,
    pub prompt_bytes: usize,
    pub output: String,
    pub note_id: u64,
    pub settled: bool,
    pub merkle_root: String,
    pub effects_count: usize,
    /// Transaction ID if submitted on-chain (base58).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_id: Option<String>,
    /// Whether the transaction was accepted on-chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_accepted: Option<bool>,
}

#[derive(Serialize, Deserialize)]
pub struct ProveResponse {
    pub query: String,
    pub chunks_retrieved: usize,
    pub retrievals: Vec<RetrievalInfo>,
    pub prompt_bytes: usize,
    pub output: String,
    pub note_id: u64,
    pub settled: bool,
    pub merkle_root: String,
    /// STARK proof bytes (hex-encoded JAM of the proof noun).
    pub proof_jam_hex: String,
    /// Size of the proof in bytes.
    pub proof_bytes: usize,
    /// Error message if prove-computation crashed (settlement did NOT happen).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prove_error: Option<String>,
    /// Transaction ID if submitted on-chain (base58).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_id: Option<String>,
    /// Whether the transaction was accepted on-chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_accepted: Option<bool>,
}

#[derive(Serialize, Deserialize)]
pub struct RetrievalInfo {
    pub chunk_id: u64,
    pub score: f64,
    pub preview: String,
}

#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub has_tree: bool,
    pub chunk_count: usize,
    pub merkle_root: Option<String>,
    pub notes_settled: u64,
    pub hull_id: u64,
    pub settlement_mode: String,
    pub recent_notes: Vec<NoteSummary>,
}

#[derive(Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Serialize)]
pub struct DiagSigHashResponse {
    /// Did %diag-cue succeed (CUE without sieve)?
    pub cue_ok: bool,
    /// Did %diag-sieve succeed (CUE + sieve inside mule)?
    pub sieve_ok: Option<bool>,
    /// Which step crashed in %diag-hash? "ok", "fail-sig-hashable", or "fail-hash-hashable".
    pub diag_hash_result: Option<String>,
    /// Did %sig-hash succeed with 5 entries (no proof)?
    pub sig_hash_5_ok: Option<bool>,
    /// Did %sig-hash succeed with 6 entries (with proof)?
    pub sig_hash_6_ok: Option<bool>,
    /// Number of note-data entries in the test seeds.
    pub note_data_entries: usize,
    /// JAM'd seeds size in bytes.
    pub seeds_jam_bytes: usize,
    /// Error details if any step failed.
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Input limits (V-C04)
// ---------------------------------------------------------------------------

/// Maximum documents per /ingest request.
const MAX_DOCUMENTS: usize = 500;
/// Maximum size of a single document in bytes (1 MB).
const MAX_DOCUMENT_BYTES: usize = 1_000_000;
/// Maximum total chunks after paragraph splitting.
const MAX_CHUNKS: usize = 50_000;
/// Maximum top_k for retrieval queries.
const MAX_TOP_K: usize = 100;
/// Maximum STARK proof size for on-chain embedding (~3.5 MB).
/// gRPC tonic default is 4 MB with zero overrides in Nockchain.
const MAX_PROOF_BYTES: usize = 3_500_000;

// ---------------------------------------------------------------------------
// Auth middleware (V-C01)
// ---------------------------------------------------------------------------

/// Set at startup when `--no-auth` is passed. Replaces the previous
/// `unsafe { env::set_var() }` pattern (V-N01).
static NO_AUTH: AtomicBool = AtomicBool::new(false);

/// API key authentication middleware (C-004).
///
/// Checks `Authorization: Bearer <key>` against VESL_API_KEY env var.
/// /health is always exempt (liveness probes).
///
/// Auth is required by default. To skip, pass `--no-auth` at startup.
async fn check_api_key(
    req: axum::extract::Request,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    if req.uri().path() == "/health" {
        return Ok(next.run(req).await);
    }

    // --no-auth disables auth entirely (C-004: explicit opt-out only)
    if NO_AUTH.load(Ordering::Relaxed) {
        return Ok(next.run(req).await);
    }

    let expected = match std::env::var("VESL_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => return Err(StatusCode::UNAUTHORIZED),
    };

    let provided = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    match provided {
        Some(token) if token == expected => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Pre-flight auth check (C-004). Call before starting the server.
/// Returns an error message if auth is misconfigured.
pub fn check_auth_config(no_auth: bool) -> Result<(), String> {
    if no_auth {
        NO_AUTH.store(true, Ordering::Relaxed);
        return Ok(());
    }
    match std::env::var("VESL_API_KEY") {
        Ok(k) if !k.is_empty() => Ok(()),
        _ => Err(
            "VESL_API_KEY is not set. Either set it or pass --no-auth for local dev.\n\
             Example: VESL_API_KEY=mysecret hull-rag --serve"
                .into(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the axum router with all Vesl API endpoints.
pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/ingest", post(ingest_handler))
        .route("/query", post(query_handler))
        .route("/prove", post(prove_handler))
        .route("/diag-sig-hash", get(diag_sig_hash_handler))
        .layer(
            tower::ServiceBuilder::new()
                .layer(axum::error_handling::HandleErrorLayer::new(|_: tower::BoxError| async {
                    StatusCode::TOO_MANY_REQUESTS
                }))
                .buffer(256)
                .rate_limit(200, std::time::Duration::from_secs(60)),
        )
        .layer(RequestBodyLimitLayer::new(4 * 1024 * 1024)) // 4 MB hard limit (H-001)
        .layer(middleware::from_fn(check_api_key)) // V-C01: API key auth
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
    })
}

async fn status(State(state): State<SharedState>) -> Json<StatusResponse> {
    let st = state.inner.lock().await;
    let merkle_root = st.tree.as_ref().map(|t| crate::merkle::format_tip5(&t.root()));
    Json(StatusResponse {
        has_tree: st.tree.is_some(),
        chunk_count: st.chunks.len(),
        merkle_root,
        notes_settled: st.note_counter,
        hull_id: st.hull_id,
        settlement_mode: st.settlement.mode.to_string(),
        recent_notes: st.recent_notes.iter().cloned().collect(),
    })
}

async fn ingest_handler(
    State(state): State<SharedState>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestResponse>, (StatusCode, Json<ErrorBody>)> {
    if req.documents.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "documents array must not be empty".into(),
            }),
        ));
    }

    // V-C04: Enforce input bounds
    if req.documents.len() > MAX_DOCUMENTS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!("too many documents ({}, max {})", req.documents.len(), MAX_DOCUMENTS),
            }),
        ));
    }
    for (i, doc) in req.documents.iter().enumerate() {
        if doc.len() > MAX_DOCUMENT_BYTES {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: format!("document {} too large ({} bytes, max {})", i, doc.len(), MAX_DOCUMENT_BYTES),
                }),
            ));
        }
    }

    // Split each document into paragraph chunks
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut next_id: u64 = 0;

    for doc in &req.documents {
        let paragraphs: Vec<String> = doc
            .split("\n\n")
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();

        for para in paragraphs {
            chunks.push(Chunk {
                id: next_id,
                dat: para,
            });
            next_id += 1;
        }
    }

    if chunks.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "no non-empty paragraphs found in documents".into(),
            }),
        ));
    }

    // V-C04: Cap total chunk count to prevent OOM during tree build
    if chunks.len() > MAX_CHUNKS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!("too many chunks ({}, max {})", chunks.len(), MAX_CHUNKS),
            }),
        ));
    }

    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = MerkleTree::build(&leaf_data);
    let root = tree.root();
    let root_hex = crate::merkle::format_tip5(&root);
    let chunk_count = chunks.len();

    // Register root with kernel
    let mut st = state.inner.lock().await;
    let register_poke = noun_builder::build_register_poke(st.hull_id, &root);
    let _effects = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        st.app.poke(SystemWire.to_wire(), register_poke),
    )
    .await
    .map_err(|_| {
        eprintln!("kernel register poke timed out");
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorBody {
                error: "kernel operation timed out".into(),
            }),
        )
    })?
    .map_err(|e| {
        let root_hex = crate::merkle::format_tip5(&root);
        eprintln!("kernel register poke failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!(
                    "register poke failed for root {} — likely cause: \
                     kernel crashed on duplicate root or malformed payload: {e}",
                    root_hex,
                ),
            }),
        )
    })?;

    st.chunks = chunks;
    st.tree = Some(tree);

    Ok(Json(IngestResponse {
        chunk_count,
        merkle_root: root_hex,
        status: "ingested".into(),
    }))
}

async fn query_handler(
    State(state): State<SharedState>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorBody>)> {
    // --- Phase 1: read state under lock, build retrievals + prompt ---
    let (retrievals, retrieval_infos, prompt, prompt_bytes, root, hull_id, hits_len, retrieval_digest) = {
        let st = state.inner.lock().await;

        let tree = st.tree.as_ref().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "no documents ingested — call POST /ingest first".into(),
                }),
            )
        })?;

        let root = tree.root();
        let k = req.top_k.unwrap_or(st.top_k).min(MAX_TOP_K); // V-C04

        // Retrieve
        let hits = st.retriever.retrieve(&req.query, &st.chunks, k);
        if hits.is_empty() {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: "no relevant chunks found for query".into(),
                }),
            ));
        }

        // Validate retriever indices before use
        for h in &hits {
            if h.chunk_index >= st.chunks.len() {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: format!("retriever returned invalid index {}", h.chunk_index),
                    }),
                ));
            }
        }

        let retrieval_infos: Vec<RetrievalInfo> = hits
            .iter()
            .map(|h| {
                let dat = &st.chunks[h.chunk_index].dat;
                RetrievalInfo {
                    chunk_id: st.chunks[h.chunk_index].id,
                    score: h.score,
                    preview: if dat.len() > 80 {
                        format!("{}...", char_safe_prefix(dat, 80))
                    } else {
                        dat.clone()
                    },
                }
            })
            .collect();

        let retrieved_chunks: Vec<&Chunk> =
            hits.iter().map(|h| &st.chunks[h.chunk_index]).collect();

        let retrievals: Vec<Retrieval> = hits
            .iter()
            .map(|h| Retrieval {
                chunk: st.chunks[h.chunk_index].clone(),
                proof: tree.proof(h.chunk_index),
                score: h.score_fixed(),
            })
            .collect();

        // C-003: hash the retrieval set for TOCTOU detection across phases
        let retrieval_digest = hash_retrievals(&retrievals);

        let prompt = llm::build_prompt(&req.query, &retrieved_chunks);
        let prompt_bytes = prompt.len();
        let hits_len = hits.len();

        (retrievals, retrieval_infos, prompt, prompt_bytes, root, st.hull_id, hits_len, retrieval_digest)
    }; // lock dropped here

    // --- Phase 2: LLM inference without lock (V-003b) ---
    let output = state.llm.generate(&prompt).await.map_err(|e| {
        eprintln!("LLM inference failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "inference failed".into(),
            }),
        )
    })?;

    // --- Phase 3: settle under lock ---
    let mut st = state.inner.lock().await;

    // TOCTOU guard: verify tree root hasn't changed during LLM inference
    if st.tree.as_ref().map(|t| t.root()) != Some(root) {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "tree changed during inference — retry query".into(),
            }),
        ));
    }

    // C-003: verify retrieval set integrity — catch mutations that preserved the root
    // but changed chunk data (e.g. concurrent /ingest with same-root re-ingestion)
    if hash_retrievals(&retrievals) != retrieval_digest {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "retrieval set changed during inference — retry query".into(),
            }),
        ));
    }

    // Build manifest
    let manifest = Manifest {
        query: req.query.clone(),
        results: retrievals,
        prompt,
        output: output.clone(),
        page: 0,
    };

    // Create note + settle via kernel poke
    // H-005: derive note ID from hash(query + timestamp + nonce) instead of counter
    let note_id = derive_note_id(&req.query);
    st.note_counter += 1;
    save_note_counter(&st.output_dir, st.note_counter);

    let note = Note {
        id: note_id,
        hull: hull_id,
        root,
        state: NoteState::Pending,
    };

    let settle_poke = noun_builder::build_settle_poke(&note, &manifest, &root);
    let effects = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        st.app.poke(SystemWire.to_wire(), settle_poke),
    )
    .await
    .map_err(|_| {
        eprintln!("kernel settle poke timed out");
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorBody {
                error: "kernel operation timed out".into(),
            }),
        )
    })?
    .map_err(|e| {
        let root_hex = crate::merkle::format_tip5(&root);
        eprintln!("kernel settle poke failed for note {note_id}: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!(
                    "settle poke failed for note {note_id} (root {}) — \
                     likely cause: root not registered or manifest hash mismatch: {e}",
                    root_hex,
                ),
            }),
        )
    })?;

    let root_hex = crate::merkle::format_tip5(&root);

    // --- On-chain submission (if settlement config allows it) ---
    let mut tx_id: Option<String> = None;
    let mut tx_accepted: Option<bool> = None;

    if let (Some(_), Some(sk)) = (&st.settlement.chain_endpoint, &st.settlement.signing_key)
    {
        if st.settlement.can_submit() {
            let note_for_settlement = Note {
                id: note_id,
                hull: hull_id,
                root,
                state: NoteState::Settled,
            };
            let settlement_data = chain::SettlementData::from_settlement(
                &note_for_settlement,
                &manifest,
                None,
            );
            let pkh = signing::pubkey_hash(&signing::derive_pubkey(sk));
            let pkh_b58 = pkh.to_base58();

            if let Some(chain_config) = st.settlement.chain_config() {
            if let Ok(mut client) = chain::ChainClient::connect(chain_config.into()).await {
                let balance = client
                    .get_balance_by_pkh(&pkh_b58, st.settlement.coinbase_timelock_min)
                    .await;

                if let Ok(ref bal) = balance {
                    let utxos = chain::extract_spendable_utxos(bal);
                    if let Some(utxo) = utxos.iter().max_by_key(|u| u.amount) {
                        let params = tx_builder::SettlementTxParams {
                            input_name: nockchain_types::tx_engine::common::Name::new(
                                utxo.name.clone(),
                                utxo.last_name.clone(),
                            ),
                            input_note_hash: utxo.last_name.clone(),
                            input_amount: utxo.amount,
                            is_coinbase: true,
                            coinbase_timelock_min: st.settlement.coinbase_timelock_min,
                            source_hash: nockchain_types::tx_engine::common::Hash::from_limbs(
                                &[0, 0, 0, 0, 0],
                            ),
                            recipient_pkh: pkh,
                            settlement: settlement_data,
                            fee: st.settlement.tx_fee,
                            signing_key: *sk,
                        };

                        if let Ok(raw_tx) =
                            tx_builder::build_settlement_tx(&mut st.app, &params).await
                        {
                            let id_b58 = raw_tx.id.to_base58();
                            tx_id = Some(id_b58.clone());
                            match client.submit_and_wait(raw_tx, &id_b58).await {
                                Ok(accepted) => tx_accepted = Some(accepted),
                                Err(_) => tx_accepted = Some(false),
                            }
                        }
                    }
                }
            }
            }
        }
    }

    // Record in recent notes ring buffer
    let query_preview = if req.query.len() > 60 {
        format!("{}...", char_safe_prefix(&req.query, 60))
    } else {
        req.query.clone()
    };
    st.recent_notes.push_back(NoteSummary {
        note_id,
        root: root_hex.clone(),
        query_preview,
        settled: true,
    });
    if st.recent_notes.len() > MAX_RECENT_NOTES {
        st.recent_notes.pop_front();
    }

    Ok(Json(QueryResponse {
        query: req.query,
        chunks_retrieved: hits_len,
        retrievals: retrieval_infos,
        prompt_bytes,
        output,
        note_id,
        settled: true,
        merkle_root: root_hex,
        effects_count: effects.len(),
        tx_id,
        tx_accepted,
    }))
}

/// Render a Nock noun as debug text, extracting tapes and cords.
fn render_noun_debug(noun: nockapp::Noun, depth: usize) -> String {
    use nockapp::Noun;
    if depth > 8 { return "...".into(); }
    if noun.is_atom() {
        if let Ok(a) = noun.as_atom() {
            if a.as_u64() == Ok(0) { return "0".into(); }
            let bytes = a.as_ne_bytes();
            let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
            if len > 0 && len < 200 {
                if let Ok(s) = std::str::from_utf8(&bytes[..len]) {
                    if s.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                        return format!("'{}'", s);
                    }
                }
            }
            if len <= 8 {
                let mut val: u64 = 0;
                for (i, &b) in bytes[..len].iter().enumerate() {
                    val |= (b as u64) << (i * 8);
                }
                return format!("{}", val);
            }
            return format!("@{}bytes", len);
        }
        "?atom".into()
    } else if let Ok(cell) = noun.as_cell() {
        let head: Noun = cell.head();
        let tail: Noun = cell.tail();
        // Check for tape (list of small atoms)
        let mut chars = Vec::new();
        let mut walk: Noun = noun;
        let mut is_tape = true;
        while let Ok(c) = walk.as_cell() {
            let h: Noun = c.head();
            if let Ok(ha) = h.as_atom() {
                if let Ok(v) = ha.as_u64() {
                    if v > 0 && v < 128 { chars.push(v as u8 as char); } else { is_tape = false; break; }
                } else { is_tape = false; break; }
            } else { is_tape = false; break; }
            walk = c.tail();
        }
        if is_tape && !chars.is_empty() && walk.is_atom() {
            return format!("\"{}\"", chars.iter().collect::<String>());
        }
        let h = render_noun_debug(head, depth + 1);
        let t = render_noun_debug(tail, depth + 1);
        format!("[{} {}]", h, t)
    } else {
        "?".into()
    }
}

async fn prove_handler(
    State(state): State<SharedState>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<ProveResponse>, (StatusCode, Json<ErrorBody>)> {
    // --- Phase 1: read state under lock, build retrievals + prompt ---
    let (retrievals, retrieval_infos, prompt, prompt_bytes, root, hull_id, hits_len, retrieval_digest) = {
        let st = state.inner.lock().await;

        // Pre-flight: reject if stack is too small for STARK proving (~3GB needed)
        if matches!(st.stack_size, NockStackSize::Tiny | NockStackSize::Small | NockStackSize::Normal | NockStackSize::Medium) {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorBody {
                    error: format!(
                        "STARK proving requires --stack-size large (4GB) or larger. \
                         Current stack: {:?}. Restart hull with: hull --stack-size large --serve",
                        st.stack_size
                    ),
                }),
            ));
        }

        let tree = st.tree.as_ref().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "no documents ingested — call POST /ingest first".into(),
                }),
            )
        })?;

        let root = tree.root();
        let k = req.top_k.unwrap_or(st.top_k).min(MAX_TOP_K); // V-C04

        // Retrieve
        let hits = st.retriever.retrieve(&req.query, &st.chunks, k);
        if hits.is_empty() {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: "no relevant chunks found for query".into(),
                }),
            ));
        }

        // Validate retriever indices before use
        for h in &hits {
            if h.chunk_index >= st.chunks.len() {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: format!("retriever returned invalid index {}", h.chunk_index),
                    }),
                ));
            }
        }

        let retrieval_infos: Vec<RetrievalInfo> = hits
            .iter()
            .map(|h| {
                let dat = &st.chunks[h.chunk_index].dat;
                RetrievalInfo {
                    chunk_id: st.chunks[h.chunk_index].id,
                    score: h.score,
                    preview: if dat.len() > 80 {
                        format!("{}...", char_safe_prefix(dat, 80))
                    } else {
                        dat.clone()
                    },
                }
            })
            .collect();

        let retrieved_chunks: Vec<&Chunk> =
            hits.iter().map(|h| &st.chunks[h.chunk_index]).collect();

        let retrievals: Vec<Retrieval> = hits
            .iter()
            .map(|h| Retrieval {
                chunk: st.chunks[h.chunk_index].clone(),
                proof: tree.proof(h.chunk_index),
                score: h.score_fixed(),
            })
            .collect();

        // C-003: hash the retrieval set for TOCTOU detection across phases
        let retrieval_digest = hash_retrievals(&retrievals);

        let prompt = llm::build_prompt(&req.query, &retrieved_chunks);
        let prompt_bytes = prompt.len();
        let hits_len = hits.len();

        (retrievals, retrieval_infos, prompt, prompt_bytes, root, st.hull_id, hits_len, retrieval_digest)
    }; // lock dropped here

    // --- Phase 2: LLM inference without lock (V-003b) ---
    let output = state.llm.generate(&prompt).await.map_err(|e| {
        eprintln!("LLM inference failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "inference failed".into(),
            }),
        )
    })?;

    // --- Phase 3: prove + settle under lock ---
    let mut st = state.inner.lock().await;

    // TOCTOU guard: verify tree root hasn't changed during LLM inference
    if st.tree.as_ref().map(|t| t.root()) != Some(root) {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "tree changed during inference — retry query".into(),
            }),
        ));
    }

    // C-003: verify retrieval set integrity
    if hash_retrievals(&retrievals) != retrieval_digest {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "retrieval set changed during inference — retry query".into(),
            }),
        ));
    }

    // Build manifest
    let manifest = Manifest {
        query: req.query.clone(),
        results: retrievals,
        prompt,
        output: output.clone(),
        page: 0,
    };

    // Create note + prove via kernel poke (%prove instead of %settle)
    // H-005: derive note ID from hash(query + timestamp + nonce) instead of counter
    let note_id = derive_note_id(&req.query);
    st.note_counter += 1;
    save_note_counter(&st.output_dir, st.note_counter);

    let note = Note {
        id: note_id,
        hull: hull_id,
        root,
        state: NoteState::Pending,
    };

    let prove_poke = noun_builder::build_prove_poke(&note, &manifest, &root);
    eprintln!("[prove] firing %prove poke (note_id={note_id})...");
    let prove_start = std::time::Instant::now();
    let effects = tokio::time::timeout(
        std::time::Duration::from_secs(600),  // STARK proving is slow
        st.app.poke(SystemWire.to_wire(), prove_poke),
    )
    .await
    .map_err(|_| {
        eprintln!("[prove] kernel %prove poke timed out");
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorBody {
                error: "prove operation timed out".into(),
            }),
        )
    })?
    .map_err(|e| {
        let root_hex = crate::merkle::format_tip5(&root);
        eprintln!("[prove] kernel %prove poke ERRORED for note {note_id}: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!(
                    "prove poke failed for note {note_id} (root {}) — \
                     likely cause: root not registered, manifest mismatch, \
                     or insufficient stack size for STARK proving: {e}",
                    root_hex,
                ),
            }),
        )
    })?;
    let prove_elapsed = prove_start.elapsed();
    eprintln!("[prove] poke returned {} effect(s) in {:.2}s", effects.len(), prove_elapsed.as_secs_f64());

    // Extract proof from effects.
    //
    // On success: ~[[result-note proof]] — one effect, a cell of [note proof]
    // On proof failure: ~[[%prove-failed ~]] — mule caught the crash, no settlement
    // On total crash: 0 effects — wrapper swallowed crash (pre-mule kernels)
    let mut proof_jam_hex = String::new();
    let mut proof_bytes_raw: Option<bytes::Bytes> = None;
    let mut prove_error: Option<String> = None;
    let mut settled = false;

    if let Some(effect_slab) = effects.first() {
        let root_noun = slab_root(effect_slab);
        eprintln!("[prove] effect root: is_cell={}", root_noun.is_cell());

        // Check for [%prove-failed ~] — head is the cord "prove-failed"
        let is_prove_failed = root_noun.as_cell().ok().and_then(|cell| {
            cell.head().as_atom().ok().and_then(|a| {
                let bytes = a.as_ne_bytes();
                let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
                if &bytes[..len] == b"prove-failed" { Some(()) } else { None }
            })
        }).is_some();

        if is_prove_failed {
            // Extract trace from [%prove-failed trace-jam]
            // trace-jam is a jammed noun containing the mule crash trace
            let tail_info = root_noun.as_cell().ok().map(|cell| {
                let tail = cell.tail();
                // The tail is (jam (list tank)) — CUE and walk the noun tree
                if tail.is_atom() {
                    let a = tail.as_atom().unwrap();
                    let bytes = a.as_ne_bytes();
                    let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
                    let _ = std::fs::write("/tmp/prove_trace.bin", &bytes[..len]);
                    eprintln!("[prove] trace atom: {} bytes", len);
                    // CUE the jammed trace
                    let mut cue_slab: NounSlab = NounSlab::new();
                    match cue_slab.cue_into(bytes::Bytes::copy_from_slice(&bytes[..len])) {
                        Ok(cued) => {
                            // Walk the list and render each tank
                            let mut msg_parts: Vec<String> = Vec::new();
                            let mut list = cued;
                            for _ in 0..20 {
                                if list.is_atom() { break; }
                                if let Ok(cell) = list.as_cell() {
                                    let tank = cell.head();
                                    // Render tank: try to extract text
                                    let rendered = render_noun_debug(tank, 0);
                                    msg_parts.push(rendered);
                                    list = cell.tail();
                                }
                            }
                            if msg_parts.is_empty() {
                                format!("trace(cued): empty list")
                            } else {
                                format!("trace(cued): {}", msg_parts.join(" | "))
                            }
                        }
                        Err(e) => {
                            // Not a jammed noun — try as UTF-8
                            eprintln!("[prove] CUE failed: {e}");
                            if let Ok(s) = std::str::from_utf8(&bytes[..len]) {
                                return format!("trace: {}", s);
                            }
                            format!("trace-raw: {} bytes", len)
                        }
                    }
                } else {
                    // Tail is a cell — walk it directly as (list tank)
                    let mut msg_parts: Vec<String> = Vec::new();
                    let mut list = tail;
                    for _ in 0..20 {
                        if list.is_atom() { break; }
                        if let Ok(cell) = list.as_cell() {
                            let tank = cell.head();
                            let rendered = render_noun_debug(tank, 0);
                            msg_parts.push(rendered);
                            list = cell.tail();
                        }
                    }
                    if msg_parts.is_empty() {
                        "trace: empty".to_string()
                    } else {
                        format!("trace(cell): {}", msg_parts.join(" | "))
                    }
                }
            }).unwrap_or_else(|| "unknown".to_string());
            eprintln!("[prove] kernel returned %prove-failed — proof crashed, note NOT settled");
            eprintln!("[prove] {}", tail_info);
            prove_error = Some(
                format!(
                    "prove-computation crashed in kernel ({}). \
                     Note was NOT settled.",
                    tail_info
                ),
            );
        } else {
            match root_noun.as_cell() {
                Ok(cell) => {
                    let proof_noun = cell.tail();
                    eprintln!(
                        "[prove] proof noun: is_cell={}, is_atom={}",
                        proof_noun.is_cell(),
                        proof_noun.is_atom()
                    );
                    let mut proof_slab: NounSlab<NockJammer> = NounSlab::new();
                    proof_slab.copy_into(proof_noun);
                    let proof_bytes = proof_slab.jam();
                    eprintln!("[prove] proof jammed: {} bytes", proof_bytes.len());
                    proof_bytes_raw = Some(proof_bytes.clone());
                    proof_jam_hex = hex::encode(&proof_bytes);
                    settled = true;
                }
                Err(_) => {
                    if root_noun.is_atom() {
                        let val = root_noun.as_atom().map(|a| a.as_u64());
                        eprintln!("[prove] effect is unexpected atom: {:?}", val);
                    }
                    prove_error = Some("unexpected effect structure from %prove poke".into());
                }
            }
        }
    } else {
        eprintln!("[prove] WARNING: 0 effects — %prove crashed (pre-mule kernel or fatal error)");
        prove_error = Some(
            "prove poke returned 0 effects — kernel crashed. \
             Recompile kernel and use --stack-size large."
                .into(),
        );
    }

    let proof_bytes_len = proof_jam_hex.len() / 2;
    let root_hex = crate::merkle::format_tip5(&root);

    // If proof failed, return error — no chain submission
    if let Some(ref err) = prove_error {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: err.clone(),
            }),
        ));
    }

    // --- On-chain submission (only if proof succeeded) ---
    let mut tx_id: Option<String> = None;
    let mut tx_accepted: Option<bool> = None;

    if let (Some(_), Some(sk)) = (&st.settlement.chain_endpoint, &st.settlement.signing_key)
    {
        if st.settlement.can_submit() {
            let note_for_settlement = Note {
                id: note_id,
                hull: hull_id,
                root,
                state: NoteState::Settled,
            };
            let proof_for_chain = proof_bytes_raw.clone().filter(|b| {
                if b.len() > MAX_PROOF_BYTES {
                    eprintln!(
                        "[prove] proof too large for on-chain: {} bytes (max {})",
                        b.len(),
                        MAX_PROOF_BYTES
                    );
                    false
                } else {
                    true
                }
            });
            let settlement_data = chain::SettlementData::from_settlement(
                &note_for_settlement,
                &manifest,
                proof_for_chain,
            );
            let pkh = signing::pubkey_hash(&signing::derive_pubkey(sk));
            let pkh_b58 = pkh.to_base58();

            if let Some(chain_config) = st.settlement.chain_config() {
            match chain::ChainClient::connect(chain_config.into()).await {
                Ok(mut client) => {
                    let balance = client
                        .get_balance_by_pkh(&pkh_b58, st.settlement.coinbase_timelock_min)
                        .await;

                    match balance {
                        Ok(ref bal) => {
                            let utxos = chain::extract_spendable_utxos(bal);
                            eprintln!("[prove] chain: {} spendable UTXOs", utxos.len());
                            if let Some(utxo) = utxos.iter().max_by_key(|u| u.amount) {
                                let params = tx_builder::SettlementTxParams {
                                    input_name: nockchain_types::tx_engine::common::Name::new(
                                        utxo.name.clone(),
                                        utxo.last_name.clone(),
                                    ),
                                    input_note_hash: utxo.last_name.clone(),
                                    input_amount: utxo.amount,
                                    is_coinbase: true,
                                    coinbase_timelock_min: st.settlement.coinbase_timelock_min,
                                    source_hash: nockchain_types::tx_engine::common::Hash::from_limbs(
                                        &[0, 0, 0, 0, 0],
                                    ),
                                    recipient_pkh: pkh,
                                    settlement: settlement_data,
                                    fee: st.settlement.tx_fee,
                                    signing_key: *sk,
                                };

                                match tx_builder::build_settlement_tx(&mut st.app, &params).await {
                                    Ok(raw_tx) => {
                                        let id_b58 = raw_tx.id.to_base58();
                                        eprintln!("[prove] tx built: {}", id_b58);
                                        tx_id = Some(id_b58.clone());
                                        match client.submit_and_wait(raw_tx, &id_b58).await {
                                            Ok(accepted) => {
                                                eprintln!("[prove] tx accepted: {}", accepted);
                                                tx_accepted = Some(accepted);
                                            }
                                            Err(e) => {
                                                eprintln!("[prove] tx submit error: {e}");
                                                tx_accepted = Some(false);
                                            }
                                        }
                                    }
                                    Err(e) => eprintln!("[prove] tx build error: {e}"),
                                }
                            }
                        }
                        Err(e) => eprintln!("[prove] balance query error: {e}"),
                    }
                }
                Err(e) => eprintln!("[prove] chain connect error: {e}"),
            }
            } else {
                eprintln!("[prove] no chain config");
            }
        }
    }

    // Record in recent notes ring buffer
    if settled {
        let query_preview = if req.query.len() > 60 {
            format!("{}...", char_safe_prefix(&req.query, 60))
        } else {
            req.query.clone()
        };
        st.recent_notes.push_back(NoteSummary {
            note_id,
            root: root_hex.clone(),
            query_preview,
            settled: true,
        });
        if st.recent_notes.len() > MAX_RECENT_NOTES {
            st.recent_notes.pop_front();
        }
    }

    Ok(Json(ProveResponse {
        query: req.query,
        chunks_retrieved: hits_len,
        retrievals: retrieval_infos,
        prompt_bytes,
        output,
        note_id,
        settled,
        merkle_root: root_hex,
        proof_jam_hex,
        proof_bytes: proof_bytes_len,
        prove_error: None,
        tx_id,
        tx_accepted,
    }))
}

// ---------------------------------------------------------------------------
// Diagnostic endpoint — %sig-hash crash isolation
// ---------------------------------------------------------------------------

/// Fire diagnostic pokes to isolate the %sig-hash sieve crash.
///
/// Constructs mock settlement seeds with 6 note-data entries (including proof)
/// and fires %diag-cue, %diag-sieve, and %sig-hash in sequence.
async fn diag_sig_hash_handler(
    State(state): State<SharedState>,
) -> Result<Json<DiagSigHashResponse>, (StatusCode, Json<ErrorBody>)> {
    use nockchain_types::tx_engine::common::{Hash, Nicks};
    use nockchain_types::tx_engine::v1::tx::{Seed, Seeds};

    let mut errors = Vec::new();

    // Build mock settlement with proof (6 entries)
    let mock_proof = bytes::Bytes::from(vec![0xABu8; 256]);
    let settlement = chain::SettlementData {
        version: 2,
        hull_id: 7,
        merkle_root: [100, 200, 300, 400, 500],
        note_id: 42,
        manifest_hash: [10, 20, 30, 40, 50],
        proof_jam: Some(mock_proof),
    };
    let note_data = tx_builder::settlement_to_note_data(&settlement);
    let note_data_entries = note_data.0.len();

    let seed = Seed {
        output_source: None,
        lock_root: Hash::from_limbs(&[1, 2, 3, 4, 5]),
        note_data,
        gift: Nicks(62_536),
        parent_hash: Hash::from_limbs(&[10, 20, 30, 40, 50]),
    };
    let seeds = Seeds(vec![seed]);
    let fee = Nicks(3000);

    // Get JAM size for diagnostics
    let seeds_jam_bytes = match tx_builder::jam_seeds_for_diag(&seeds) {
        Ok(b) => b.len(),
        Err(e) => {
            errors.push(format!("jam_seeds failed: {e}"));
            0
        }
    };

    let mut st = state.inner.lock().await;

    // 1. %diag-cue — CUE without sieve
    let cue_ok = match tx_builder::kernel_diag_cue(&mut st.app, &seeds).await {
        Ok(effects) => {
            eprintln!("[diag] cue: {} effects returned", effects.len());
            !effects.is_empty()
        }
        Err(e) => {
            errors.push(format!("diag-cue: {e}"));
            false
        }
    };

    // 2. %diag-sieve — CUE + sieve inside mule
    let sieve_ok = if cue_ok {
        match tx_builder::kernel_diag_sieve(&mut st.app, &seeds).await {
            Ok(effects) => {
                if let Some(effect) = effects.first() {
                    let root = unsafe { *effect.root() };
                    // Effect is [%diag-sieve result ...]. Check if result is %ok.
                    if let Ok(cell) = root.as_cell() {
                        if let Ok(inner) = cell.tail().as_cell() {
                            let tag = inner.head();
                            let is_ok = tag.as_atom().map(|a| a.as_u64() == Ok(nockvm_macros::tas!(b"ok") as u64)).unwrap_or(false);
                            if !is_ok {
                                errors.push("diag-sieve: sieve FAILED (;;(seeds:txv1 ...) crashed)".into());
                            }
                            eprintln!("[diag] sieve: ok={is_ok}");
                            Some(is_ok)
                        } else {
                            errors.push("diag-sieve: unexpected effect shape".into());
                            Some(false)
                        }
                    } else {
                        errors.push("diag-sieve: effect is not a cell".into());
                        Some(false)
                    }
                } else {
                    errors.push("diag-sieve: no effects (poke crashed before mule)".into());
                    Some(false)
                }
            }
            Err(e) => {
                errors.push(format!("diag-sieve: {e}"));
                Some(false)
            }
        }
    } else {
        None
    };

    // 3. %diag-hash — isolate crash in sig-hashable vs hash-hashable
    let diag_hash_result = match tx_builder::kernel_diag_hash(&mut st.app, &seeds, &fee).await {
        Ok(effects) => {
            if let Some(effect) = effects.first() {
                let root = unsafe { *effect.root() };
                if let Ok(cell) = root.as_cell() {
                    if let Ok(inner) = cell.tail().as_cell() {
                        let tag = inner.head();
                        let tag_str = if let Ok(a) = tag.as_atom() {
                            if a.as_u64() == Ok(nockvm_macros::tas!(b"ok") as u64) {
                                "ok".to_string()
                            } else if a.as_u64() == Ok(nockvm_macros::tas!(b"fail") as u64) {
                                "fail".to_string()
                            } else {
                                format!("unknown-tag-{:?}", a.as_u64())
                            }
                        } else {
                            "non-atom-tag".to_string()
                        };
                        eprintln!("[diag] hash: {tag_str}");
                        if tag_str != "ok" {
                            errors.push(format!("diag-hash: {tag_str}"));
                        }
                        Some(tag_str)
                    } else { Some("bad-shape".to_string()) }
                } else { Some("not-cell".to_string()) }
            } else {
                errors.push("diag-hash: no effects (poke crashed before mule)".into());
                Some("no-effects".to_string())
            }
        }
        Err(e) => {
            errors.push(format!("diag-hash: {e}"));
            Some(format!("error: {e}"))
        }
    };

    // 4. %sig-hash with 5 entries (no proof) — control test
    let settlement_no_proof = chain::SettlementData {
        version: 1,
        hull_id: 7,
        merkle_root: [100, 200, 300, 400, 500],
        note_id: 42,
        manifest_hash: [10, 20, 30, 40, 50],
        proof_jam: None,
    };
    let nd5 = tx_builder::settlement_to_note_data(&settlement_no_proof);
    let seed5 = Seed {
        output_source: None,
        lock_root: Hash::from_limbs(&[1, 2, 3, 4, 5]),
        note_data: nd5,
        gift: Nicks(62_536),
        parent_hash: Hash::from_limbs(&[10, 20, 30, 40, 50]),
    };
    let seeds5 = Seeds(vec![seed5]);

    let sig_hash_5_ok = match tx_builder::kernel_sig_hash(&mut st.app, &seeds5, &fee).await {
        Ok(_hash) => {
            eprintln!("[diag] sig-hash (5 entries, no proof): OK");
            Some(true)
        }
        Err(e) => {
            errors.push(format!("sig-hash-5: {e}"));
            eprintln!("[diag] sig-hash (5 entries): FAILED — {e}");
            Some(false)
        }
    };

    // 5. %sig-hash with 6 entries (with proof) — the crash case
    let sig_hash_ok = match tx_builder::kernel_sig_hash(&mut st.app, &seeds, &fee).await {
        Ok(_hash) => {
            eprintln!("[diag] sig-hash (6 entries, with proof): OK");
            Some(true)
        }
        Err(e) => {
            errors.push(format!("sig-hash-6: {e}"));
            eprintln!("[diag] sig-hash (6 entries): FAILED — {e}");
            Some(false)
        }
    };

    Ok(Json(DiagSigHashResponse {
        cue_ok,
        sieve_ok,
        diag_hash_result,
        sig_hash_5_ok,
        sig_hash_6_ok: sig_hash_ok,
        note_data_entries,
        seeds_jam_bytes,
        errors,
    }))
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Start the HTTP API server on the given port.
pub async fn serve(state: SharedState, port: u16, bind_addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("{bind_addr}:{port}")).await?;
    if std::env::var("VESL_API_KEY").map_or(true, |k| k.is_empty()) {
        eprintln!("WARNING: VESL_API_KEY not set — API endpoints are unauthenticated (V-C01)");
    }
    println!("Vesl Hull API listening on http://{bind_addr}:{port}");
    println!("  POST /ingest  — upload documents");
    println!("  POST /query   — retrieve + infer + settle");
    println!("  POST /prove   — retrieve + infer + settle + STARK proof (needs --stack-size large)");
    println!("  GET  /status  — current state");
    println!("  GET  /health  — liveness check");
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use clap::Parser;
    use nockapp::kernel::boot;
    use tower::ServiceExt;

    use crate::config::SettlementConfig;
    use crate::retrieve::KeywordRetriever;

    /// Create a test AppState with the real kernel.
    async fn test_state() -> SharedState {
        // Disable auth for unit tests (mirrors --no-auth at startup)
        check_auth_config(true).ok();

        // Parse from empty args to get all defaults
        let cli = boot::Cli::parse_from(["test", "--new"]);
        let app: NockApp =
            boot::setup(kernels_vesl::KERNEL, cli, &[], "vesl", None)
                .await
                .expect("kernel boot");

        Arc::new(ServerState {
            inner: Mutex::new(AppState {
                app,
                chunks: Vec::new(),
                tree: None,
                hull_id: 7,
                top_k: 2,
                retriever: Box::new(KeywordRetriever),
                note_counter: 0,
                settlement: SettlementConfig::local(),
                stack_size: NockStackSize::Normal,
                output_dir: std::env::temp_dir(),
                recent_notes: std::collections::VecDeque::new(),
            }),
            llm: Box::new(llm::StubProvider),
        })
    }

    /// Helper: collect response body bytes.
    async fn body_bytes(resp: axum::http::Response<Body>) -> Vec<u8> {
        use http_body_util::BodyExt;
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let state = test_state().await;
        let app = router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let bytes = body_bytes(resp).await;
        assert_eq!(status, StatusCode::OK);
        let json: HealthResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json.status, "ok");
    }

    #[tokio::test]
    async fn status_empty_state() {
        let state = test_state().await;
        let app = router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let bytes = body_bytes(resp).await;
        assert_eq!(status, StatusCode::OK);
        let json: StatusResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(!json.has_tree);
        assert_eq!(json.chunk_count, 0);
        assert!(json.merkle_root.is_none());
    }

    #[tokio::test]
    async fn ingest_creates_tree() {
        let state = test_state().await;
        let app = router(state.clone());

        let req_body = serde_json::json!({
            "documents": ["First paragraph.\n\nSecond paragraph."]
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let bytes = body_bytes(resp).await;
        assert_eq!(status, StatusCode::OK);
        let json: IngestResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json.chunk_count, 2);
        assert_eq!(json.status, "ingested");
        assert!(!json.merkle_root.is_empty());

        // Verify state updated
        let st = state.inner.lock().await;
        assert_eq!(st.chunks.len(), 2);
        assert!(st.tree.is_some());
    }

    #[tokio::test]
    async fn ingest_empty_documents_rejected() {
        let state = test_state().await;
        let app = router(state);

        let req_body = serde_json::json!({ "documents": [] });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn query_without_ingest_rejected() {
        let state = test_state().await;
        let app = router(state);

        let req_body = serde_json::json!({ "query": "test query" });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/query")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn full_ingest_then_query() {
        let state = test_state().await;

        // Step 1: Ingest
        let ingest_body = serde_json::json!({
            "documents": [
                "Q3 revenue: $4.2M ARR, 18% QoQ growth\n\nRisk exposure: $800K in variable-rate instruments",
                "Board approved Series B at $45M pre-money\n\nSOC2 Type II audit scheduled for Q4"
            ]
        });

        let app = router(state.clone());
        let resp = app
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

        // Step 2: Query
        let query_body = serde_json::json!({
            "query": "Q3 revenue growth"
        });

        let app = router(state.clone());
        let resp = app
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

        let status = resp.status();
        let bytes = body_bytes(resp).await;
        assert_eq!(status, StatusCode::OK);
        let json: QueryResponse = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json.query, "Q3 revenue growth");
        assert!(json.chunks_retrieved > 0);
        assert!(json.settled);
        // note_id is now derived from timestamp+entropy (anti-replay),
        // so just assert it's non-zero rather than a sequential index.
        assert!(json.note_id > 0);
        assert!(!json.merkle_root.is_empty());
        assert!(json.prompt_bytes > 0);
        assert!(!json.output.is_empty());
    }

    #[tokio::test]
    async fn oversized_body_rejected_413() {
        let state = test_state().await;
        let app = router(state);

        // 11 MB body — exceeds the 10 MB hard limit
        let big_body = vec![b'x'; 11 * 1024 * 1024];

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest")
                    .header("content-type", "application/json")
                    .body(Body::from(big_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    // AUDIT 2026-04-19 H-09 regression: byte-slicing these previews panicked
    // on multi-byte UTF-8 whose truncation point fell mid-codepoint. The
    // helper now splits on char boundaries.
    #[test]
    fn char_safe_prefix_ascii_exact() {
        assert_eq!(char_safe_prefix("hello world", 5), "hello");
    }

    #[test]
    fn char_safe_prefix_shorter_than_cap() {
        assert_eq!(char_safe_prefix("abc", 10), "abc");
    }

    #[test]
    fn char_safe_prefix_three_byte_chars() {
        // Forty 3-byte CJK chars = 120 bytes. Byte index 60 is mid-codepoint
        // (byte 60 of 0xE4 0xB8 0xAD is the middle of one char) and would
        // panic the old &s[..60] slice. We take 40 chars' worth of the
        // 40-char input, which is all of it.
        let s = "中".repeat(40);
        let out = char_safe_prefix(&s, 40);
        assert_eq!(out, s);
        // Ask for fewer chars than the input has and assert boundary.
        let out = char_safe_prefix(&s, 20);
        assert_eq!(out.chars().count(), 20);
        assert!(s.is_char_boundary(out.len()));
    }

    #[test]
    fn char_safe_prefix_emoji_boundary() {
        // A 78-byte ASCII run + 4-byte emoji + tail. Byte 80 is inside the
        // emoji; the old slice would panic. Ask for 79 chars (ASCII run -
        // one) to prove we stop cleanly before the multi-byte codepoint.
        let s = format!("{}🔒{}", "a".repeat(78), "b".repeat(20));
        assert!(s.len() > 80);
        let out = char_safe_prefix(&s, 79);
        assert_eq!(out.chars().count(), 79);
        assert!(s.is_char_boundary(out.len()));
        // Slicing at 80 chars consumes the emoji; make sure no panic.
        let out = char_safe_prefix(&s, 80);
        assert_eq!(out.chars().count(), 80);
        assert!(s.is_char_boundary(out.len()));
    }
}
