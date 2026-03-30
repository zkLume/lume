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

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use nockchain_math::belt::Belt;

use crate::chain;
use crate::llm::{self, LlmProvider};
use crate::merkle::MerkleTree;
use crate::noun_builder;
use crate::retrieve::Retriever;
use crate::signing;
use crate::tx_builder;
use crate::types::*;

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
    pub llm: Box<dyn LlmProvider>,
    pub retriever: Box<dyn Retriever + Send + Sync>,
    /// Count of settled notes (incremented per successful /query).
    pub note_counter: u64,
    /// Chain endpoint for on-chain settlement submission.
    pub chain_endpoint: Option<String>,
    /// Signing key for settlement transactions.
    pub signing_key: Option<[Belt; 8]>,
    /// Coinbase timelock minimum.
    pub coinbase_timelock_min: u64,
    /// Transaction fee.
    pub tx_fee: u64,
}

pub type SharedState = Arc<Mutex<AppState>>;

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
}

#[derive(Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
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
    let st = state.lock().await;
    let merkle_root = st.tree.as_ref().map(|t| crate::merkle::format_tip5(&t.root()));
    Json(StatusResponse {
        has_tree: st.tree.is_some(),
        chunk_count: st.chunks.len(),
        merkle_root,
        notes_settled: st.note_counter,
        hull_id: st.hull_id,
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

    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = MerkleTree::build(&leaf_data);
    let root = tree.root();
    let root_hex = crate::merkle::format_tip5(&root);
    let chunk_count = chunks.len();

    // Register root with kernel
    let mut st = state.lock().await;
    let register_poke = noun_builder::build_register_poke(st.hull_id, &root);
    let _effects = st
        .app
        .poke(SystemWire.to_wire(), register_poke)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: format!("kernel register poke failed: {e}"),
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
    let mut st = state.lock().await;

    let tree = st.tree.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "no documents ingested — call POST /ingest first".into(),
            }),
        )
    })?;

    let root = tree.root();
    let k = req.top_k.unwrap_or(st.top_k);

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

    let retrieval_infos: Vec<RetrievalInfo> = hits
        .iter()
        .map(|h| {
            let dat = &st.chunks[h.chunk_index].dat;
            RetrievalInfo {
                chunk_id: st.chunks[h.chunk_index].id,
                score: h.score,
                preview: if dat.len() > 80 {
                    format!("{}...", &dat[..80])
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

    // Build prompt + call LLM
    let prompt = llm::build_prompt(&req.query, &retrieved_chunks);
    let prompt_bytes = prompt.len();

    let output = st.llm.generate(&prompt).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("LLM inference failed: {e}"),
            }),
        )
    })?;

    // Build manifest
    let manifest = Manifest {
        query: req.query.clone(),
        results: retrievals,
        prompt,
        output: output.clone(),
    };

    // Create note + settle via kernel poke
    st.note_counter += 1;
    let note_id = st.note_counter;

    let note = Note {
        id: note_id,
        hull: st.hull_id,
        root,
        state: NoteState::Pending,
    };

    let settle_poke = noun_builder::build_settle_poke(&note, &manifest, &root);
    let effects = st
        .app
        .poke(SystemWire.to_wire(), settle_poke)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: format!("kernel settle poke failed: {e}"),
                }),
            )
        })?;

    let root_hex = crate::merkle::format_tip5(&root);

    // --- On-chain submission (if chain endpoint + signing key configured) ---
    let mut tx_id: Option<String> = None;
    let mut tx_accepted: Option<bool> = None;

    if let (Some(endpoint), Some(sk)) = (&st.chain_endpoint, &st.signing_key) {
        let note_for_settlement = Note {
            id: note_id,
            hull: st.hull_id,
            root,
            state: NoteState::Settled,
        };
        let settlement = chain::SettlementData::from_settlement(&note_for_settlement, &manifest);
        let pkh = signing::pubkey_hash(&signing::derive_pubkey(sk));
        let pkh_b58 = pkh.to_base58();

        let chain_config = chain::ChainConfig::local(endpoint);
        if let Ok(mut client) = chain::ChainClient::connect(chain_config).await {
            let balance = client
                .get_balance_by_pkh(&pkh_b58, st.coinbase_timelock_min)
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
                        coinbase_timelock_min: st.coinbase_timelock_min,
                        source_hash: nockchain_types::tx_engine::common::Hash::from_limbs(
                            &[0, 0, 0, 0, 0],
                        ),
                        recipient_pkh: pkh,
                        settlement,
                        fee: st.tx_fee,
                        signing_key: *sk,
                    };

                    if let Ok(raw_tx) = tx_builder::build_settlement_tx(&mut st.app, &params).await
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

    Ok(Json(QueryResponse {
        query: req.query,
        chunks_retrieved: hits.len(),
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

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Start the HTTP API server on the given port.
pub async fn serve(state: SharedState, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    println!("Vesl Hull API listening on http://0.0.0.0:{port}");
    println!("  POST /ingest  — upload documents");
    println!("  POST /query   — retrieve + infer + settle");
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

    use crate::retrieve::KeywordRetriever;

    /// Create a test AppState with the real kernel.
    async fn test_state() -> SharedState {
        // Parse from empty args to get all defaults
        let cli = boot::Cli::parse_from(["test", "--new"]);
        let app: NockApp =
            boot::setup(kernels_vesl::KERNEL, cli, &[], "vesl", None)
                .await
                .expect("kernel boot");

        Arc::new(Mutex::new(AppState {
            app,
            chunks: Vec::new(),
            tree: None,
            hull_id: 7,
            top_k: 2,
            llm: Box::new(llm::StubProvider),
            retriever: Box::new(KeywordRetriever),
            note_counter: 0,
            chain_endpoint: None,
            signing_key: None,
            coinbase_timelock_min: 1,
            tx_fee: 3000,
        }))
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
        let st = state.lock().await;
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
        assert_eq!(json.note_id, 1);
        assert!(!json.merkle_root.is_empty());
        assert!(json.prompt_bytes > 0);
        assert!(!json.output.is_empty());
    }
}
