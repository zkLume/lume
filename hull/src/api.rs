//! HTTP API — axum server for the generic hull.
//!
//! Three domain endpoints: /commit, /settle, /verify.
//! Plus /health and /status for ops.
//!
//! Community developers: modify /commit to accept your domain data,
//! adjust the Merkle leaf encoding, and add domain-specific endpoints.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::limit::RequestBodyLimitLayer;

use vesl_core::{format_tip5, MerkleTree};

use crate::config::SettlementConfig;
use crate::verify::field_to_leaf_bytes;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// A key-value field. The atomic unit of committed data.
/// Community developers: replace this with your domain primitive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Shared state for the HTTP API.
///
/// Held behind `Arc<Mutex<...>>`. No LLM, no retriever — a single
/// mutex is sufficient since no handler blocks for long.
pub struct AppState {
    pub app: NockApp,
    pub fields: Vec<Field>,
    pub tree: Option<MerkleTree>,
    pub hull_id: u64,
    pub note_counter: u64,
    pub settlement: SettlementConfig,
    pub output_dir: PathBuf,
}

pub type SharedState = Arc<Mutex<AppState>>;

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
    // AUDIT 2026-04-17 L-05: atomic write via tempfile + rename. Does
    // not prevent read-modify-write races between two hull processes
    // sharing `output_dir` (still a single-writer invariant by
    // design), but eliminates torn writes if the process is killed
    // mid-write or if two writers race by coincidence.
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
pub struct CommitRequest {
    pub fields: Vec<Field>,
}

#[derive(Serialize)]
pub struct CommitResponse {
    pub field_count: usize,
    pub merkle_root: String,
    pub status: String,
}

#[derive(Deserialize)]
pub struct SettleRequest {
    /// Optional note ID. Auto-increments if omitted.
    pub note_id: Option<u64>,
}

#[derive(Serialize)]
pub struct SettleResponse {
    pub note_id: u64,
    pub merkle_root: String,
    pub settled: bool,
    pub effects_count: usize,
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub field: Field,
    /// Hex-encoded tip5 Merkle root to verify against.
    pub merkle_root: String,
}

#[derive(Serialize)]
pub struct VerifyResponse {
    pub valid: bool,
    pub field_key: String,
    pub merkle_root: String,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub has_tree: bool,
    pub field_count: usize,
    pub merkle_root: Option<String>,
    pub notes_settled: u64,
    pub hull_id: u64,
    pub settlement_mode: String,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

// ---------------------------------------------------------------------------
// Input limits
// ---------------------------------------------------------------------------

/// Maximum fields per /commit request.
const MAX_FIELDS: usize = 500;
/// Maximum size of a single field key or value in bytes.
const MAX_FIELD_BYTES: usize = 100_000;

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

/// Set at startup when `--no-auth` is passed. Replaces the previous
/// `unsafe { env::set_var() }` pattern (V-N01).
static NO_AUTH: AtomicBool = AtomicBool::new(false);

/// API key authentication middleware (C-004).
///
/// Checks `Authorization: Bearer <key>` against HULL_API_KEY env var.
/// /health is always exempt.
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

    let expected = match std::env::var("HULL_API_KEY") {
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
///
/// Loopback-only entry point. Production callers should use
/// `check_auth_config_with_bind` so the M-15 non-loopback refusal runs.
pub fn check_auth_config(no_auth: bool) -> Result<(), String> {
    check_auth_config_with_bind(no_auth, "127.0.0.1")
}

/// Variant used by the CLI entry point — knows the bind address, so
/// it can reject `--no-auth` on non-loopback binds.
///
/// AUDIT 2026-04-19 M-15: `--no-auth` on an exposed bind leaks state
/// and lets anyone poke the kernel. Fail-closed when `no_auth` is set
/// AND `bind_addr` isn't loopback.
pub fn check_auth_config_with_bind(no_auth: bool, bind_addr: &str) -> Result<(), String> {
    if no_auth {
        if !is_loopback_bind(bind_addr) {
            return Err(format!(
                "--no-auth on bind address `{bind_addr}` is refused. \
                 --no-auth is only permitted on loopback binds (127.0.0.1, ::1, localhost). \
                 Set HULL_API_KEY and drop --no-auth, or change bind-addr to loopback."
            ));
        }
        NO_AUTH.store(true, Ordering::Relaxed);
        return Ok(());
    }
    match std::env::var("HULL_API_KEY") {
        Ok(k) if !k.is_empty() => Ok(()),
        _ => Err(
            "HULL_API_KEY is not set. Either set it or pass --no-auth for local dev.\n\
             Example: HULL_API_KEY=mysecret hull --port 3000"
                .into(),
        ),
    }
}

fn is_loopback_bind(bind_addr: &str) -> bool {
    let host = bind_addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(bind_addr);
    let host = host.trim_matches(|c| c == '[' || c == ']');
    matches!(host, "127.0.0.1" | "::1" | "localhost")
        || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the axum router with all hull endpoints.
pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/commit", post(commit_handler))
        .route("/settle", post(settle_handler))
        .route("/verify", post(verify_handler))
        .layer(
            tower::ServiceBuilder::new()
                .layer(axum::error_handling::HandleErrorLayer::new(|_: tower::BoxError| async {
                    StatusCode::TOO_MANY_REQUESTS
                }))
                .buffer(256)
                .rate_limit(200, std::time::Duration::from_secs(60)),
        )
        .layer(RequestBodyLimitLayer::new(4 * 1024 * 1024)) // H-001
        .layer(middleware::from_fn(check_api_key))
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
    let merkle_root = st.tree.as_ref().map(|t| format_tip5(&t.root()));
    Json(StatusResponse {
        has_tree: st.tree.is_some(),
        field_count: st.fields.len(),
        merkle_root,
        notes_settled: st.note_counter,
        hull_id: st.hull_id,
        settlement_mode: st.settlement.mode.to_string(),
    })
}

/// POST /commit — accept fields, build Merkle tree, register root.
async fn commit_handler(
    State(state): State<SharedState>,
    Json(req): Json<CommitRequest>,
) -> Result<Json<CommitResponse>, (StatusCode, Json<ErrorBody>)> {
    if req.fields.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "fields array must not be empty".into(),
            }),
        ));
    }

    if req.fields.len() > MAX_FIELDS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!("too many fields ({}, max {})", req.fields.len(), MAX_FIELDS),
            }),
        ));
    }

    for (i, field) in req.fields.iter().enumerate() {
        if field.key.len() > MAX_FIELD_BYTES || field.value.len() > MAX_FIELD_BYTES {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: format!("field {} too large (max {} bytes per key/value)", i, MAX_FIELD_BYTES),
                }),
            ));
        }
        if field.key.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: format!("field {} has empty key", i),
                }),
            ));
        }
    }

    // Build Merkle tree from field data
    let leaf_data: Vec<Vec<u8>> = req.fields.iter().map(|f| field_to_leaf_bytes(f)).collect();
    let leaf_refs: Vec<&[u8]> = leaf_data.iter().map(|v| v.as_slice()).collect();
    let tree = MerkleTree::build(&leaf_refs);
    let root = tree.root();
    let root_hex = format_tip5(&root);
    let field_count = req.fields.len();

    // Register root with kernel
    let mut st = state.lock().await;
    let register_poke = vesl_core::noun_builder::build_register_poke(st.hull_id, &root);
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
        eprintln!("kernel register poke failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "internal error".into(),
            }),
        )
    })?;

    st.fields = req.fields;
    st.tree = Some(tree);

    Ok(Json(CommitResponse {
        field_count,
        merkle_root: root_hex,
        status: "committed".into(),
    }))
}

/// POST /settle — settle a note against the current Merkle root.
async fn settle_handler(
    State(state): State<SharedState>,
    Json(req): Json<SettleRequest>,
) -> Result<Json<SettleResponse>, (StatusCode, Json<ErrorBody>)> {
    let mut st = state.lock().await;

    let tree = st.tree.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "no tree committed yet -- POST /commit first".into(),
            }),
        )
    })?;

    let root = tree.root();
    let root_hex = format_tip5(&root);

    // Increment note counter
    st.note_counter += 1;
    let note_id = req.note_id.unwrap_or(st.note_counter);
    save_note_counter(&st.output_dir, st.note_counter);

    // Poke kernel with register (settlement primitive for generic hull)
    let settle_poke = vesl_core::noun_builder::build_register_poke(st.hull_id, &root);
    let effects = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        st.app.poke(SystemWire.to_wire(), settle_poke),
    )
    .await
    .map_err(|_| {
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorBody {
                error: "kernel operation timed out".into(),
            }),
        )
    })?
    .map_err(|e| {
        eprintln!("kernel settle poke failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "internal error".into(),
            }),
        )
    })?;

    Ok(Json(SettleResponse {
        note_id,
        merkle_root: root_hex,
        settled: !effects.is_empty(),
        effects_count: effects.len(),
    }))
}

/// POST /verify — verify a field's commitment against a Merkle root.
async fn verify_handler(
    State(state): State<SharedState>,
    Json(req): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, (StatusCode, Json<ErrorBody>)> {
    let st = state.lock().await;

    let tree = st.tree.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "no tree committed yet -- POST /commit first".into(),
            }),
        )
    })?;

    let root = tree.root();
    let current_root_hex = format_tip5(&root);

    // If the caller provided a specific root, verify against that
    let target_root_hex = if req.merkle_root.is_empty() {
        current_root_hex.clone()
    } else {
        req.merkle_root.clone()
    };

    // Find the field in committed fields
    let leaf_bytes = field_to_leaf_bytes(&req.field);
    let position = st.fields.iter().position(|f| {
        f.key == req.field.key && f.value == req.field.value
    });

    let valid = match position {
        Some(idx) => {
            let proof = tree.proof(idx);
            // Verify against current root (the only one we have locally)
            nockchain_tip5_rs::verify_proof(&leaf_bytes, &proof, &root)
                && target_root_hex == current_root_hex
        }
        None => false,
    };

    Ok(Json(VerifyResponse {
        valid,
        field_key: req.field.key,
        merkle_root: target_root_hex,
    }))
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Start the HTTP server.
pub async fn serve(state: SharedState, port: u16, bind_addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("{bind_addr}:{port}")).await?;
    if std::env::var("HULL_API_KEY").map_or(true, |k| k.is_empty()) {
        eprintln!("WARNING: HULL_API_KEY not set -- API endpoints are unauthenticated");
    }
    println!("Hull API listening on http://{bind_addr}:{port}");
    println!("  POST /commit  -- commit key-value fields");
    println!("  POST /settle  -- settle a note");
    println!("  POST /verify  -- verify a field commitment");
    println!("  GET  /status  -- current state");
    println!("  GET  /health  -- liveness check");
    axum::serve(listener, app).await?;
    Ok(())
}
