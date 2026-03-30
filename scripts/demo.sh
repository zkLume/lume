#!/usr/bin/env bash
# ============================================================================
# Vesl Live Demo — Full Pipeline on Nockchain Fakenet
#
# A single command that demonstrates the entire Vesl pipeline:
#   document ingestion → retrieval → LLM inference → ZK settlement →
#   chain submission → on-chain confirmation
#
# Usage:
#   ./scripts/demo.sh              # Full demo (boots fakenet, runs pipeline)
#   ./scripts/demo.sh --no-fakenet # Skip fakenet boot (use running instance)
#   ./scripts/demo.sh --no-chain   # Local-only (no chain interaction)
#   ./scripts/demo.sh --ollama-url http://host:11434  # Use real LLM
#
# Prerequisites:
#   - nockchain binary in PATH
#   - cargo (Rust toolchain)
#   - curl
#   - Optional: Ollama instance for real LLM inference
# ============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HULL_DIR="$PROJECT_ROOT/hull"
DEMO_DOCS="$PROJECT_ROOT/demo/docs"

# ---------------------------------------------------------------------------
# Configuration (override via environment or flags)
# ---------------------------------------------------------------------------

MANAGE_FAKENET=true
USE_CHAIN=true
OLLAMA_URL="${OLLAMA_URL:-}"
OLLAMA_MODEL="${OLLAMA_MODEL:-llama3.2}"
HULL_PORT="${HULL_PORT:-3000}"
NOCKCHAIN_GRPC_ADDR="${NOCKCHAIN_GRPC_ADDR:-127.0.0.1:9090}"
# Demo signing key PKH — matches hull::signing::demo_signing_key()
MINING_PKH="${MINING_PKH:-5pJiNWqnouxku6SvGU6XZhu98nHH5VFMaNJ4r1vtHxPJ5sHurHBfYnk}"
WAIT_BLOCKS_TIMEOUT="${WAIT_BLOCKS_TIMEOUT:-120}"
HULL_PID=""

# Parse flags
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-fakenet)  MANAGE_FAKENET=false; shift ;;
        --no-chain)    USE_CHAIN=false; MANAGE_FAKENET=false; shift ;;
        --ollama-url)  OLLAMA_URL="$2"; shift 2 ;;
        --ollama-model) OLLAMA_MODEL="$2"; shift 2 ;;
        --port)        HULL_PORT="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --no-fakenet              Skip fakenet boot (use running instance)"
            echo "  --no-chain                Local-only demo (no chain interaction)"
            echo "  --ollama-url URL          Use real LLM (e.g., http://localhost:11434)"
            echo "  --ollama-model NAME       Ollama model name (default: llama3.2)"
            echo "  --port PORT               Hull HTTP port (default: 3000)"
            echo "  -h, --help                Show this help"
            echo ""
            echo "Environment:"
            echo "  OLLAMA_URL                Same as --ollama-url"
            echo "  MINING_PKH                Miner PKH for fakenet (base58)"
            echo "  NOCKCHAIN_GRPC_ADDR       Chain gRPC address (default: 127.0.0.1:9090)"
            echo "  WAIT_BLOCKS_TIMEOUT       Seconds to wait for mined blocks (default: 120)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            echo "Run '$0 --help' for usage." >&2
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Output formatting
# ---------------------------------------------------------------------------

BOLD='\033[1m'
DIM='\033[2m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
RESET='\033[0m'

banner() { echo -e "\n${BOLD}${CYAN}=== $* ===${RESET}\n"; }
step()   { echo -e "${BOLD}[$1]${RESET} $2"; }
ok()     { echo -e "    ${GREEN}$*${RESET}"; }
warn()   { echo -e "    ${YELLOW}$*${RESET}"; }
fail()   { echo -e "    ${RED}$*${RESET}"; }
dim()    { echo -e "    ${DIM}$*${RESET}"; }
field()  { printf "    ${BOLD}%-20s${RESET} %s\n" "$1" "$2"; }

# ---------------------------------------------------------------------------
# Cleanup handler
# ---------------------------------------------------------------------------

cleanup() {
    local rc=$?
    echo ""
    if [[ -n "$HULL_PID" ]] && kill -0 "$HULL_PID" 2>/dev/null; then
        dim "Stopping hull server (pid $HULL_PID)..."
        kill "$HULL_PID" 2>/dev/null || true
        # Wait up to 5s for graceful shutdown, then force kill
        local i=0
        while kill -0 "$HULL_PID" 2>/dev/null && [[ $i -lt 5 ]]; do
            sleep 1
            i=$((i + 1))
        done
        if kill -0 "$HULL_PID" 2>/dev/null; then
            dim "Force-killing hull..."
            kill -9 "$HULL_PID" 2>/dev/null || true
        fi
        wait "$HULL_PID" 2>/dev/null || true
    fi
    if [[ "$MANAGE_FAKENET" == "true" ]]; then
        dim "Stopping fakenet..."
        "$SCRIPT_DIR/fakenet-harness.sh" stop 2>/dev/null || true
    fi
    if [[ $rc -eq 0 ]]; then
        banner "Demo Complete"
    else
        banner "Demo Failed (exit code $rc)"
    fi
    exit $rc
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Prerequisite checks
# ---------------------------------------------------------------------------

banner "Vesl: Sovereign RAG on Nockchain"

step "0" "Checking prerequisites..."

if ! command -v cargo &>/dev/null; then
    fail "cargo not found. Install Rust: https://rustup.rs"
    exit 1
fi
ok "cargo: $(cargo --version | head -1)"

if ! command -v curl &>/dev/null; then
    fail "curl not found."
    exit 1
fi
ok "curl: available"

if [[ "$MANAGE_FAKENET" == "true" ]] || [[ "$USE_CHAIN" == "true" ]]; then
    if ! command -v nockchain &>/dev/null; then
        fail "nockchain not found in PATH."
        fail "Install: cd \$NOCK_HOME && make install-nockchain"
        exit 1
    fi
    ok "nockchain: available"
fi

if [[ ! -d "$DEMO_DOCS" ]]; then
    fail "Demo documents not found at $DEMO_DOCS"
    exit 1
fi
ok "demo docs: $(ls "$DEMO_DOCS"/*.txt 2>/dev/null | wc -l) file(s) in demo/docs/"

if [[ -n "$OLLAMA_URL" ]]; then
    ok "LLM: Ollama at $OLLAMA_URL (model: $OLLAMA_MODEL)"
else
    warn "LLM: stub provider (set OLLAMA_URL or --ollama-url for real inference)"
fi

# ---------------------------------------------------------------------------
# Step 1: Boot fakenet
# ---------------------------------------------------------------------------

if [[ "$MANAGE_FAKENET" == "true" ]]; then
    step "1" "Booting Nockchain fakenet..."
    "$SCRIPT_DIR/fakenet-harness.sh" start
    ok "Fakenet running (hub + miner)"
elif [[ "$USE_CHAIN" == "true" ]]; then
    step "1" "Using existing fakenet at $NOCKCHAIN_GRPC_ADDR"
    dim "(pass --no-chain to skip chain interaction)"
else
    step "1" "Local-only mode (no chain)"
fi

# ---------------------------------------------------------------------------
# Step 2: Wait for mined blocks
# ---------------------------------------------------------------------------

if [[ "$USE_CHAIN" == "true" ]]; then
    step "2" "Waiting for mined blocks..."

    GRPC_HOST="${NOCKCHAIN_GRPC_ADDR%%:*}"
    GRPC_PORT="${NOCKCHAIN_GRPC_ADDR##*:}"
    CHAIN_URL="http://$NOCKCHAIN_GRPC_ADDR"

    # Poll the gRPC port until a balance query returns notes
    elapsed=0
    has_blocks=false
    while [[ $elapsed -lt $WAIT_BLOCKS_TIMEOUT ]]; do
        # Build hull first if needed (do this while waiting)
        if [[ $elapsed -eq 0 ]]; then
            dim "Building hull while waiting for blocks..."
            (cd "$HULL_DIR" && cargo build --release 2>&1 | tail -3) || true
            ok "Hull built"
        fi

        # Try a simple gRPC connectivity check via the test binary
        if (cd "$HULL_DIR" && \
            VESL_FAKENET_CHAIN_ENDPOINT="$CHAIN_URL" \
            VESL_FAKENET_WALLET_ADDRESS="$MINING_PKH" \
            VESL_FAKENET_COINBASE_TIMELOCK_MIN=1 \
            cargo test --release fakenet_balance_query --test e2e_fakenet -- --ignored --nocapture 2>&1 \
            | grep -q "note(s) on-chain"); then
            has_blocks=true
            break
        fi

        sleep 5
        elapsed=$((elapsed + 5))
        dim "Waiting for blocks... (${elapsed}s / ${WAIT_BLOCKS_TIMEOUT}s)"
    done

    if [[ "$has_blocks" == "true" ]]; then
        ok "Miner has produced blocks with funded coinbase UTXOs"
    else
        warn "Timed out waiting for blocks (${WAIT_BLOCKS_TIMEOUT}s)"
        warn "Continuing with demo — chain queries may return empty results"
    fi
else
    step "2" "Skipping block wait (local-only mode)"
    dim "Building hull..."
    (cd "$HULL_DIR" && cargo build --release 2>&1 | tail -3)
    ok "Hull built"
fi

# ---------------------------------------------------------------------------
# Step 3: Start hull HTTP server
# ---------------------------------------------------------------------------

step "3" "Starting Vesl hull HTTP server..."

mkdir -p "$PROJECT_ROOT/.fakenet"

HULL_BIN="$HULL_DIR/target/release/hull"
if [[ ! -x "$HULL_BIN" ]]; then
    # Fallback to debug build
    HULL_BIN="$HULL_DIR/target/debug/hull"
    if [[ ! -x "$HULL_BIN" ]]; then
        fail "Hull binary not found. Build with: cd hull && cargo build --release"
        exit 1
    fi
fi

# Construct hull flags
HULL_FLAGS=(
    --new
    --serve
    --port "$HULL_PORT"
    --docs "$DEMO_DOCS"
    --top-k 3
)

if [[ -n "$OLLAMA_URL" ]]; then
    HULL_FLAGS+=(--ollama-url "$OLLAMA_URL" --model "$OLLAMA_MODEL")
fi

if [[ "$USE_CHAIN" == "true" ]]; then
    HULL_FLAGS+=(
        --chain-endpoint "http://$NOCKCHAIN_GRPC_ADDR"
        --wallet-address "$MINING_PKH"
        --submit
        --coinbase-timelock-min 1
    )
fi

dim "Command: hull ${HULL_FLAGS[*]}"

# Start hull in background
"$HULL_BIN" "${HULL_FLAGS[@]}" > "$PROJECT_ROOT/.fakenet/hull.log" 2>&1 &
HULL_PID=$!
dim "Hull PID: $HULL_PID, log: $PROJECT_ROOT/.fakenet/hull.log"

# Wait for the HTTP server to be ready
HULL_URL="http://localhost:$HULL_PORT"
hull_ready=false
for i in $(seq 1 60); do
    if curl -sf "$HULL_URL/health" >/dev/null 2>&1; then
        hull_ready=true
        break
    fi
    sleep 1
done

if [[ "$hull_ready" == "true" ]]; then
    ok "Hull HTTP server ready at $HULL_URL"
else
    fail "Hull server failed to start within 60s"
    fail "Check log: $PROJECT_ROOT/.fakenet/hull.log"
    cat "$PROJECT_ROOT/.fakenet/hull.log" | tail -20
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 4: Check ingestion status (docs pre-loaded via --docs)
# ---------------------------------------------------------------------------

step "4" "Checking document ingestion..."

STATUS=$(curl -sf "$HULL_URL/status")
CHUNK_COUNT=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin)['chunk_count'])" 2>/dev/null || echo "0")
MERKLE_ROOT=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('merkle_root','none'))" 2>/dev/null || echo "none")
HAS_TREE=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin)['has_tree'])" 2>/dev/null || echo "false")

if [[ "$HAS_TREE" == "True" ]] || [[ "$HAS_TREE" == "true" ]]; then
    ok "Documents ingested via --docs pre-loading"
    field "Chunks:" "$CHUNK_COUNT"
    field "Merkle root:" "${MERKLE_ROOT:0:32}..."
else
    # Fallback: ingest via API
    dim "Pre-loading did not run. Ingesting via API..."

    INGEST_BODY='{"documents":['
    first=true
    for f in "$DEMO_DOCS"/*.txt; do
        content=$(python3 -c "import json,sys; print(json.dumps(open(sys.argv[1]).read()))" "$f")
        if [[ "$first" == "true" ]]; then
            first=false
        else
            INGEST_BODY+=","
        fi
        INGEST_BODY+="$content"
    done
    INGEST_BODY+=']}'

    INGEST_RESP=$(curl -sf -X POST "$HULL_URL/ingest" \
        -H "Content-Type: application/json" \
        -d "$INGEST_BODY")

    CHUNK_COUNT=$(echo "$INGEST_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['chunk_count'])" 2>/dev/null || echo "?")
    MERKLE_ROOT=$(echo "$INGEST_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['merkle_root'])" 2>/dev/null || echo "?")

    ok "Ingested $CHUNK_COUNT chunks"
    field "Merkle root:" "${MERKLE_ROOT:0:32}..."
fi

echo ""
dim "Documents ingested:"
for f in "$DEMO_DOCS"/*.txt; do
    dim "  - $(basename "$f") ($(wc -c < "$f") bytes)"
done

# ---------------------------------------------------------------------------
# Step 5: Query — retrieve, infer, settle
# ---------------------------------------------------------------------------

banner "RAG Query Pipeline"

QUERY="What is the company Q3 revenue and what are the key financial risks?"

step "5" "Querying: \"$QUERY\""
echo ""

QUERY_BODY=$(python3 -c "import json,sys; print(json.dumps({'query': sys.argv[1], 'top_k': 3}))" "$QUERY")

QUERY_RESP=$(curl -sf -X POST "$HULL_URL/query" \
    -H "Content-Type: application/json" \
    -d "$QUERY_BODY")

# Parse response
Q_QUERY=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['query'])" 2>/dev/null)
Q_CHUNKS=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['chunks_retrieved'])" 2>/dev/null)
Q_OUTPUT=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['output'])" 2>/dev/null)
Q_SETTLED=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['settled'])" 2>/dev/null)
Q_NOTE_ID=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['note_id'])" 2>/dev/null)
Q_ROOT=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['merkle_root'])" 2>/dev/null)
Q_PROMPT_BYTES=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['prompt_bytes'])" 2>/dev/null)
Q_EFFECTS=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['effects_count'])" 2>/dev/null)
Q_TX_ID=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tx_id','none'))" 2>/dev/null || echo "none")
Q_TX_ACCEPTED=$(echo "$QUERY_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tx_accepted','none'))" 2>/dev/null || echo "none")

# Print retrieved chunks
RETRIEVALS=$(echo "$QUERY_RESP" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for r in data.get('retrievals', []):
    print(f\"  chunk[{r['chunk_id']}] score={r['score']:.2f}: {r['preview'][:70]}...\")
" 2>/dev/null)

echo -e "${BOLD}  Retrieved Chunks:${RESET}"
echo "$RETRIEVALS"
echo ""

echo -e "${BOLD}  LLM Response:${RESET}"
echo "$Q_OUTPUT" | fold -s -w 76 | sed 's/^/    /'
echo ""

echo -e "${BOLD}  Settlement:${RESET}"
field "Note ID:" "$Q_NOTE_ID"
field "Settled:" "$Q_SETTLED"
field "Merkle root:" "${Q_ROOT:0:32}..."
field "Prompt size:" "$Q_PROMPT_BYTES bytes"
field "Kernel effects:" "$Q_EFFECTS"
if [[ "$Q_TX_ID" != "none" ]] && [[ "$Q_TX_ID" != "None" ]]; then
    field "TX ID:" "$Q_TX_ID"
    field "TX Accepted:" "$Q_TX_ACCEPTED"
fi

# ---------------------------------------------------------------------------
# Step 6: Show kernel state
# ---------------------------------------------------------------------------

step "6" "Kernel state after settlement..."

STATUS=$(curl -sf "$HULL_URL/status")
NOTES_SETTLED=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin)['notes_settled'])" 2>/dev/null || echo "0")
HULL_ID=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin)['hull_id'])" 2>/dev/null || echo "?")

field "Hull ID:" "$HULL_ID"
field "Notes settled:" "$NOTES_SETTLED"
field "Chunks in tree:" "$CHUNK_COUNT"
field "Merkle root:" "${Q_ROOT:0:32}..."

# ---------------------------------------------------------------------------
# Step 7: On-chain state (if chain is available)
# ---------------------------------------------------------------------------

if [[ "$USE_CHAIN" == "true" ]]; then
    banner "On-Chain Settlement"

    step "7" "Querying Nockchain for settlement data..."

    # Run the chain scan via the integration test
    CHAIN_OUTPUT=$(cd "$HULL_DIR" && \
        VESL_FAKENET_CHAIN_ENDPOINT="http://$NOCKCHAIN_GRPC_ADDR" \
        VESL_FAKENET_WALLET_ADDRESS="$MINING_PKH" \
        VESL_FAKENET_COINBASE_TIMELOCK_MIN=1 \
        cargo test --release fakenet_find_settlement_notes --test e2e_fakenet -- --ignored --nocapture 2>&1 \
        || true)

    SETTLEMENT_COUNT=$(echo "$CHAIN_OUTPUT" | grep -o '[0-9]* Vesl settlement' | head -1 | grep -o '[0-9]*' || echo "0")

    if [[ "$SETTLEMENT_COUNT" -gt 0 ]]; then
        ok "Found $SETTLEMENT_COUNT Vesl settlement(s) on-chain"
        echo "$CHAIN_OUTPUT" | grep "Settlement(" | sed 's/^/    /'
    else
        dim "No Vesl settlements found on-chain via test scanner."
        if [[ "$Q_TX_ACCEPTED" == "True" ]] || [[ "$Q_TX_ACCEPTED" == "true" ]]; then
            ok "But /query reported TX accepted — settlement is on-chain!"
            field "TX ID:" "$Q_TX_ID"
        fi
    fi

    # Show chain connectivity
    echo ""
    step "8" "Chain connectivity status..."
    field "gRPC endpoint:" "http://$NOCKCHAIN_GRPC_ADDR"
    field "Mining PKH:" "${MINING_PKH:0:16}..."

    # Query balance to show chain is alive
    BALANCE_OUTPUT=$(cd "$HULL_DIR" && \
        VESL_FAKENET_CHAIN_ENDPOINT="http://$NOCKCHAIN_GRPC_ADDR" \
        VESL_FAKENET_WALLET_ADDRESS="$MINING_PKH" \
        VESL_FAKENET_COINBASE_TIMELOCK_MIN=1 \
        cargo test --release fakenet_balance_query --test e2e_fakenet -- --ignored --nocapture 2>&1 \
        || true)

    NOTE_COUNT=$(echo "$BALANCE_OUTPUT" | grep -o '[0-9]* note(s)' | head -1 | grep -o '[0-9]*' || echo "0")
    field "On-chain notes:" "$NOTE_COUNT"
else
    step "7" "Chain interaction skipped (--no-chain)"
    step "8" "Skipped"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

banner "Pipeline Summary"

echo -e "${BOLD}  What just happened:${RESET}"
echo ""
echo "  1. Ingested $(ls "$DEMO_DOCS"/*.txt | wc -l) documents into $CHUNK_COUNT chunks"
echo "  2. Built tip5 Merkle tree (root: ${Q_ROOT:0:16}...)"
echo "  3. Registered root with Hoon kernel (cryptographic commitment)"
echo "  4. Retrieved top-$Q_CHUNKS chunks via keyword scoring"
echo "  5. Built deterministic prompt ($Q_PROMPT_BYTES bytes)"
if [[ -n "$OLLAMA_URL" ]]; then
echo "  6. Generated response via Ollama ($OLLAMA_MODEL)"
else
echo "  6. Generated response via stub LLM (use --ollama-url for real inference)"
fi
echo "  7. Settled note #$Q_NOTE_ID in Hoon kernel ($Q_EFFECTS effects)"
echo "     - Merkle verification: all chunk proofs validated"
echo "     - Prompt integrity: byte-exact match confirmed"
echo "     - State transition: Pending -> Settled (or crash)"

if [[ "$USE_CHAIN" == "true" ]]; then
echo "  8. Connected to Nockchain fakenet at $NOCKCHAIN_GRPC_ADDR"
echo "     - Balance: $NOTE_COUNT note(s) at mining PKH"
if [[ "$Q_TX_ID" != "none" ]] && [[ "$Q_TX_ID" != "None" ]]; then
echo "  9. Built + signed settlement transaction (tx-id: ${Q_TX_ID:0:16}...)"
echo "     - 5 NoteData entries: vesl-v, vesl-vid, vesl-rt, vesl-nid, vesl-mh"
if [[ "$Q_TX_ACCEPTED" == "True" ]] || [[ "$Q_TX_ACCEPTED" == "true" ]]; then
echo "  10. Transaction ACCEPTED on-chain"
echo "      Settlement data is now permanently recorded on Nockchain."
else
echo "  10. Transaction submitted (acceptance pending)"
fi
fi
fi

echo ""
echo -e "${BOLD}  Security model (6 layers):${RESET}"
echo "     Mold boundary    -> structural errors caught"
echo "     Merkle verify    -> tampered chunks rejected"
echo "     Prompt rebuild   -> injection attacks blocked"
echo "     settle-note      -> crash on ANY failure (no valid STARK)"
echo "     Registration     -> uncommitted roots rejected"
echo "     Replay guard     -> duplicate settlements blocked"

echo ""
echo -e "${BOLD}  API endpoints:${RESET}"
echo "     POST /ingest  -> $HULL_URL/ingest"
echo "     POST /query   -> $HULL_URL/query"
echo "     GET  /status  -> $HULL_URL/status"
echo "     GET  /health  -> $HULL_URL/health"

echo ""
echo -e "${DIM}  Server still running at $HULL_URL (Ctrl+C or wait for cleanup)${RESET}"
echo ""
