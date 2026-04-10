#!/usr/bin/env bash
# Reproduce NockStack exhaustion: %prove → %sig-hash failure
# Run on PC only (needs 128GB for STARK proving)
#
# Prerequisites:
#   - nockchain binary in PATH
#   - hull built with diagnostic eprintln (vesl/fix-stack-reset branch in nockchain)
#   - fakenet harness available
#
# What this does:
#   1. Boots fakenet (hub + miner)
#   2. Waits for a few blocks to be mined (need spendable UTXOs)
#   3. Starts hull in serve mode with --submit --stack-size large
#   4. Ingests test data
#   5. POST /prove (triggers %prove poke → STARK proving → %sig-hash → %tx-id)
#   6. Captures diagnostic stderr output showing stack state across pokes
#   7. Tears everything down
#
# Expected failure: %sig-hash poke returns 0 effects after heavy %prove poke

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIAG_LOG="$PROJECT_ROOT/diag_output.log"

cleanup() {
    echo "=== Cleaning up ==="
    # Kill hull if running
    if [[ -n "${HULL_PID:-}" ]] && kill -0 "$HULL_PID" 2>/dev/null; then
        kill "$HULL_PID" 2>/dev/null; wait "$HULL_PID" 2>/dev/null || true
    fi
    # Stop fakenet
    "$SCRIPT_DIR/fakenet-harness.sh" stop 2>/dev/null || true
}
trap cleanup EXIT

echo "=== NockStack Diagnostic Reproduction ==="
echo "Log: $DIAG_LOG"
echo ""

# 0. Clean slate
rm -rf "$PROJECT_ROOT/.fakenet" "$PROJECT_ROOT/.data.vesl" "$DIAG_LOG"

# 1. Boot fakenet
echo "[1] Booting fakenet (hub + miner)..."
"$SCRIPT_DIR/fakenet-harness.sh" start

# 2. Wait for blocks to be mined (need coinbase UTXOs)
echo "[2] Waiting 30s for blocks to be mined..."
sleep 30
echo "    Miner log tail:"
tail -3 "$PROJECT_ROOT/.fakenet/miner.log" 2>/dev/null || echo "    (no miner log)"

# 3. Start hull in serve mode with fakenet settlement
echo "[3] Starting hull (serve + fakenet + submit + stack-size large)..."
cd "$PROJECT_ROOT"
hull/target/release/hull \
    --new --serve \
    --stack-size large \
    --settlement-mode fakenet \
    --chain-endpoint http://127.0.0.1:9090 \
    --submit \
    --coinbase-timelock-min 1 \
    2>"$DIAG_LOG" &
HULL_PID=$!
echo "    hull pid: $HULL_PID"

# Wait for hull HTTP
for i in $(seq 1 60); do
    if curl -s http://127.0.0.1:3000/health 2>/dev/null | grep -q ok; then
        echo "    hull ready after ${i}s"
        break
    fi
    sleep 1
done

# 4. Ingest
echo ""
echo "[4] Ingesting test data..."
INGEST=$(curl -s -X POST http://127.0.0.1:3000/ingest \
    -H "Content-Type: application/json" \
    -d '{"documents": ["Q3 revenue: 4.2M ARR, 18 percent QoQ growth.\n\nRisk exposure: 800K in variable-rate instruments."]}')
echo "    $INGEST" | head -1

# 5. Prove (triggers %prove → %sig-hash → %tx-id pipeline)
echo ""
echo "[5] POST /prove (STARK proving + settlement TX)..."
echo "    This will take several minutes for STARK proving."
echo "    After proof, hull will try %sig-hash and %tx-id pokes on same NockApp."
PROVE=$(curl -s --max-time 900 -X POST http://127.0.0.1:3000/prove \
    -H "Content-Type: application/json" \
    -d '{"query": "Q3 revenue", "top_k": 1}')
echo "    Prove response (summary):"
echo "$PROVE" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(f'    settled={d.get(\"settled\")}')
    print(f'    proof_bytes={d.get(\"proof_bytes\")}')
    print(f'    prove_error={d.get(\"prove_error\")}')
    print(f'    tx_id={d.get(\"tx_id\")}')
    print(f'    tx_accepted={d.get(\"tx_accepted\")}')
except: print('    (could not parse JSON)')
" 2>/dev/null

# 6. Diagnostic output
echo ""
echo "=========================================="
echo "=== DIAGNOSTIC OUTPUT (from stderr) ==="
echo "=========================================="
echo ""
cat "$DIAG_LOG"
echo ""
echo "=========================================="
echo "=== DIAG LINES ONLY ==="
echo "=========================================="
grep "DIAG" "$DIAG_LOG" || echo "(no DIAG lines found)"
echo ""
echo "=== Done ==="
