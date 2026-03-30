#!/usr/bin/env bash
# ============================================================================
# Vesl Fakenet Test Harness
#
# Boots a local Nockchain fakenet (hub + miner) and optionally runs
# the Vesl hull E2E integration tests against it.
#
# Prerequisites:
#   - nockchain binary in PATH (make install-nockchain from $NOCK_HOME)
#   - nockchain-wallet binary in PATH (make install-nockchain-wallet)
#   - hull binary built (cargo build from vesl/hull/)
#
# Usage:
#   # Boot fakenet only (keep running for manual testing):
#   ./scripts/fakenet-harness.sh start
#
#   # Run E2E tests against an already-running fakenet:
#   ./scripts/fakenet-harness.sh test
#
#   # Boot fakenet, run tests, then shut down:
#   ./scripts/fakenet-harness.sh run
#
#   # Stop all fakenet processes:
#   ./scripts/fakenet-harness.sh stop
#
#   # Show status of fakenet processes:
#   ./scripts/fakenet-harness.sh status
# ============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HULL_DIR="$PROJECT_ROOT/hull"

# Load environment
ENV_FILE="${ENV_FILE:-$SCRIPT_DIR/.env.fakenet}"
if [[ -f "$ENV_FILE" ]]; then
    # shellcheck source=/dev/null
    source "$ENV_FILE"
fi

# Defaults (can override via .env.fakenet or environment)
NOCKCHAIN_GRPC_ADDR="${NOCKCHAIN_GRPC_ADDR:-127.0.0.1:9090}"
WALLET_GRPC_ADDR="${WALLET_GRPC_ADDR:-http://localhost:5555}"
VESL_API_PORT="${VESL_API_PORT:-3000}"
# Demo signing key PKH — derived from hull::signing::demo_signing_key()
# (sk[0]=12345, sk[1]=67890). This ensures the hull can spend mined coinbase UTXOs.
MINING_PKH="${MINING_PKH:-5pJiNWqnouxku6SvGU6XZhu98nHH5VFMaNJ4r1vtHxPJ5sHurHBfYnk}"
FAKENET_POW_LEN="${FAKENET_POW_LEN:-2}"
FAKENET_LOG_DIFFICULTY="${FAKENET_LOG_DIFFICULTY:-1}"

# Working directories (state isolation)
FAKENET_DIR="$PROJECT_ROOT/.fakenet"
HUB_DIR="$FAKENET_DIR/hub"
MINER_DIR="$FAKENET_DIR/miner"

# PID files
HUB_PID="$FAKENET_DIR/hub.pid"
MINER_PID="$FAKENET_DIR/miner.pid"

# Hub multiaddr (fixed for peer discovery)
HUB_BIND="/ip4/127.0.0.1/udp/3006/quic-v1"

# ============================================================================
# Helpers
# ============================================================================

log() { echo "[fakenet] $*"; }
err() { echo "[fakenet] ERROR: $*" >&2; }

check_binary() {
    if ! command -v "$1" &>/dev/null; then
        err "$1 not found in PATH."
        err "Install it from \$NOCK_HOME: make install-$1"
        exit 1
    fi
}

is_running() {
    local pidfile="$1"
    if [[ -f "$pidfile" ]]; then
        local pid
        pid=$(cat "$pidfile")
        if kill -0 "$pid" 2>/dev/null; then
            return 0
        fi
    fi
    return 1
}

wait_for_port() {
    local host="$1" port="$2" timeout="${3:-30}"
    local elapsed=0
    while ! bash -c "echo >/dev/tcp/$host/$port" 2>/dev/null; do
        sleep 1
        elapsed=$((elapsed + 1))
        if [[ $elapsed -ge $timeout ]]; then
            err "Timeout waiting for $host:$port after ${timeout}s"
            return 1
        fi
    done
    log "$host:$port is ready (${elapsed}s)"
}

# ============================================================================
# Commands
# ============================================================================

cmd_start() {
    check_binary nockchain

    log "Starting Vesl fakenet..."
    mkdir -p "$HUB_DIR" "$MINER_DIR"

    # --- Start hub node ---
    if is_running "$HUB_PID"; then
        log "Hub already running (pid $(cat "$HUB_PID"))"
    else
        log "Starting hub node..."
        (
            cd "$HUB_DIR"
            export RUST_LOG="${RUST_LOG:-info}"
            export MINIMAL_LOG_FORMAT="${MINIMAL_LOG_FORMAT:-true}"
            nockchain \
                --fakenet \
                --bind "$HUB_BIND" \
                --bind-public-grpc-addr "$NOCKCHAIN_GRPC_ADDR" \
                --fakenet-pow-len "$FAKENET_POW_LEN" \
                --fakenet-log-difficulty "$FAKENET_LOG_DIFFICULTY" \
                > "$FAKENET_DIR/hub.log" 2>&1 &
            echo $! > "$HUB_PID"
        )
        log "Hub started (pid $(cat "$HUB_PID")), log: $FAKENET_DIR/hub.log"
    fi

    # Wait for hub gRPC
    local grpc_host grpc_port
    grpc_host="${NOCKCHAIN_GRPC_ADDR%%:*}"
    grpc_port="${NOCKCHAIN_GRPC_ADDR##*:}"
    log "Waiting for hub gRPC at $NOCKCHAIN_GRPC_ADDR..."
    wait_for_port "$grpc_host" "$grpc_port" 60

    # --- Start miner node ---
    if is_running "$MINER_PID"; then
        log "Miner already running (pid $(cat "$MINER_PID"))"
    else
        log "Starting miner node..."
        (
            cd "$MINER_DIR"
            export RUST_LOG="${RUST_LOG:-info}"
            export MINIMAL_LOG_FORMAT="${MINIMAL_LOG_FORMAT:-true}"
            nockchain \
                --mine \
                --fakenet \
                --mining-pkh "$MINING_PKH" \
                --peer "$HUB_BIND" \
                --no-default-peers \
                --fakenet-pow-len "$FAKENET_POW_LEN" \
                --fakenet-log-difficulty "$FAKENET_LOG_DIFFICULTY" \
                > "$FAKENET_DIR/miner.log" 2>&1 &
            echo $! > "$MINER_PID"
        )
        log "Miner started (pid $(cat "$MINER_PID")), log: $FAKENET_DIR/miner.log"
    fi

    # Give miner a moment to connect to hub
    sleep 3

    log "Fakenet is running."
    log "  Hub gRPC:   http://$NOCKCHAIN_GRPC_ADDR"
    log "  Hub log:    $FAKENET_DIR/hub.log"
    log "  Miner log:  $FAKENET_DIR/miner.log"
    log ""
    log "To run Vesl tests:"
    log "  ./scripts/fakenet-harness.sh test"
}

cmd_stop() {
    log "Stopping fakenet..."
    for pidfile in "$MINER_PID" "$HUB_PID"; do
        if [[ -f "$pidfile" ]]; then
            local pid name
            pid=$(cat "$pidfile")
            name=$(basename "$pidfile" .pid)
            if kill -0 "$pid" 2>/dev/null; then
                log "Stopping $name (pid $pid)..."
                kill "$pid" 2>/dev/null || true
                # Wait up to 10s for graceful shutdown
                local i=0
                while kill -0 "$pid" 2>/dev/null && [[ $i -lt 10 ]]; do
                    sleep 1
                    i=$((i + 1))
                done
                if kill -0 "$pid" 2>/dev/null; then
                    log "Force-killing $name..."
                    kill -9 "$pid" 2>/dev/null || true
                fi
            fi
            rm -f "$pidfile"
        fi
    done
    log "Fakenet stopped."
}

cmd_status() {
    log "Fakenet status:"
    for pidfile in "$HUB_PID" "$MINER_PID"; do
        local name
        name=$(basename "$pidfile" .pid)
        if is_running "$pidfile"; then
            log "  $name: running (pid $(cat "$pidfile"))"
        else
            log "  $name: stopped"
        fi
    done
}

cmd_test() {
    log "Running Vesl E2E fakenet tests..."
    log "  Chain endpoint: http://$NOCKCHAIN_GRPC_ADDR"
    log "  Wallet gRPC:    $WALLET_GRPC_ADDR"

    cd "$HULL_DIR"

    VESL_FAKENET_CHAIN_ENDPOINT="http://$NOCKCHAIN_GRPC_ADDR" \
    VESL_FAKENET_WALLET_ENDPOINT="$WALLET_GRPC_ADDR" \
    VESL_FAKENET_WALLET_ADDRESS="$MINING_PKH" \
    VESL_FAKENET_COINBASE_TIMELOCK_MIN=1 \
    cargo test --test e2e_fakenet -- --ignored --nocapture 2>&1

    log "E2E fakenet tests complete."
}

cmd_run() {
    cmd_start
    local rc=0
    cmd_test || rc=$?
    cmd_stop
    exit $rc
}

# ============================================================================
# Main
# ============================================================================

case "${1:-help}" in
    start)  cmd_start ;;
    stop)   cmd_stop ;;
    status) cmd_status ;;
    test)   cmd_test ;;
    run)    cmd_run ;;
    *)
        echo "Usage: $0 {start|stop|status|test|run}"
        echo ""
        echo "  start   Boot fakenet hub + miner (background)"
        echo "  stop    Stop all fakenet processes"
        echo "  status  Show running processes"
        echo "  test    Run E2E tests against running fakenet"
        echo "  run     Start fakenet, run tests, stop fakenet"
        exit 1
        ;;
esac
