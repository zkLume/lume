# Vesl

A verification SDK for Nockchain. Four commitment primitives — **mint** (commit), **guard** (verify), **settle** (on-chain), **forge** (STARK-prove) — each shipping as a Hoon kernel with a Rust facade in `vesl-core`, graft templates for adding them to an existing NockApp, and `hull-rag` as a reference implementation.

Grafts in Vesl fall into five families: **commitment** (shipped), **verification gates** (scaffolded library), **state** (planned), **behavior** (planned), and **intent** (placeholder, pending canonical upstream). The priority lattice in [`docs/graft-manifest.md`](docs/graft-manifest.md) is the authoritative map; the summary below groups the current primitive list the same way.


## Demo

```bash
# Reference pipeline (hull-rag) — no chain required, runs in ~30 seconds
./scripts/demo.sh --no-chain

# Full pipeline with live fakenet settlement
./scripts/demo.sh
```

Ingest documents, retrieve against a query, verify in the kernel, settle. `--no-chain` skips chain interaction.


## Structure

```
protocol/                       Hoon source
  lib/                            grafts + kernels, grouped by family (see catalog below)
    mint-kernel.hoon                family 1 — commit data → root
    guard-kernel.hoon               family 1 — verify inclusion proofs
    settle-kernel.hoon              family 1 — on-chain settlement
    forge-kernel.hoon               family 1 — STARK-prove arbitrary computation
    mint-graft.hoon                 family 1 — mint composition (priority 20)
    guard-graft.hoon                family 1 — guard composition (priority 30)
    settle-graft.hoon               family 1 — settle composition (priority 10)
    forge-graft.hoon                family 1 — forge composition (priority 40)
    intent-graft.hoon               family 5 — placeholder (priority 200, crashes on invocation)
    vesl-merkle.hoon                tip5 Merkle math
    vesl-prover.hoon                STARK proof generation
    vesl-verifier.hoon              STARK proof verification
    vesl-test.hoon                  compile-time assertions
  sur/vesl.hoon                   types

kernels/                        compiled kernel crates (one per JAM)
  mint/  guard/  settle/  forge/  vesl/

crates/                         Rust crates
  vesl-core/                      SDK — Mint/Guard/Settle/Forge facades
  nock-noun-rs/                   Nock noun construction from Rust
  nockchain-tip5-rs/              standalone tip5 Merkle tree + hashing
  nockchain-client-rs/            chain RPC client

hull-rag/                       reference implementation — verifiable RAG
  src/                            ingest, retrieve, kernel verify, settle
  tests/                          37 E2E tests (pipeline, adversarial, fakenet)

templates/                      starter NockApps + graft templates
  counter/  data-registry/  settle-report/       teach the core patterns
  graft-scaffold/  graft-mint/  graft-settle/    commitment-family grafts
  graft-hash-gate/                                custom-gate demo (formerly graft-intent)
  graft-intent/                                   family-5 placeholder stub (see MOVED.md)
  GRAFTING.md                                     long-form integration guide

assets/                         compiled kernel JAMs
demo/                           sample documents for the hull-rag pipeline
scripts/                        demo + fakenet harness
hoon/                           symlink tree (setup-hoon-tree.sh links $NOCK_HOME)
```

## Primitive families

Five graft families, one row each. Priority bands come straight from [`docs/graft-manifest.md`](docs/graft-manifest.md); see that file for the rationale behind the lattice.

| # | Family | Role | Priority band | Status | What ships today |
|---|---|---|---|---|---|
| 1 | Commitment | STARK-bearing primitives that commit data to hull-keyed roots | 10–40 | Shipped | `settle-graft` (10), `mint-graft` (20), `guard-graft` (30), `forge-graft` (40) |
| 2 | Verification gates | Parameterized decision functions consumed by commitment grafts; a library, not a priority-claimed graft | n/a (library) | Scaffolded | `vesl-gates.hoon` (planned; see `.dev/01_GATE_CATALOG.md`) |
| 3 | State | Domain-keyed app-state primitives (kv, counter, queue, rbac, registry) | 50–99 | Planned | see `.dev/02_STATE_GRAFTS.md` |
| 4 | Behavior | Runtime wrappers that enforce or observe rules around other grafts | 100–149 | Planned | see `.dev/03_BEHAVIOR_GRAFTS.md` |
| 5 | Intent | Multi-party coordination primitives (declare / match / cancel / expire) | 200–299 | Placeholder | `intent-graft` stub; crashes on invocation pending canonical upstream |

Commitments do not require intents. A NockApp can produce a ZK proof and settle it without ever declaring an intent. Intents are optional coordination on top of commitments — the STARK pipeline itself is intent-free.


## Nockchain for Rust Developers

If you're coming from EVM or Solidity, this table will save you a few hours of head-scratching.

| Nockchain | EVM equivalent |
|-----------|---------------|
| kernel | smart contract |
| hull | dApp backend / off-chain server |
| poke | transaction / state-changing call |
| peek | view / staticcall (read-only) |
| hoonc | solc (compiler) |
| NockApp | dApp |
| the subject | contract storage |
| crash | revert |
| nockvm | EVM (deterministic interpreter, but in-process — not a separate VM) |

A **kernel** is a Hoon program compiled to Nock that runs inside a NockApp. It holds state in *the subject* (think: a single persistent storage slot that's the entire state tree) and responds to pokes and peeks. Pokes mutate state and return effects; peeks read without changing anything. If something goes wrong, the kernel crashes — equivalent to a Solidity `revert`, the state change gets discarded.

The **hull** is the off-chain Rust process that hosts the kernel, handles HTTP, talks to the chain, and does the heavy lifting (Merkle trees, LLM calls, transaction construction). Kernels don't do I/O — the hull does.

**hoonc** compiles Hoon source to a `.jam` binary. No ABI, no deploy step — the compiled noun *is* the program.

If you know Foundry, think of `make demo-local` as `forge test` — it runs the full pipeline locally without touching a chain.


## Quick Start

Two ways in, depending on what you're building.

### Docker — full environment, zero setup

Use Docker if you want to run the Vesl pipeline (ingest, query, settle) or hack on the Vesl codebase itself. The container ships hoonc, nockchain, hull-rag, and all compiled kernels. Nothing to install on the host.

Pull the prebuilt image from [Docker Hub](https://hub.docker.com/r/zkvesl/vesl):

```bash
docker pull zkvesl/vesl:latest
docker run -it zkvesl/vesl:latest
make demo-local
```

Or build it locally from the `Dockerfile` in this repo: `docker build -t zkvesl/vesl:latest - < Dockerfile`.

### NockUp — add verification to your NockApp

If you're building a NockApp with `nockup` and want to graft Vesl's verification primitives onto it, head to [vesl-nockup](https://github.com/zkVesl/vesl-nockup). That repo has pre-resolved git deps, a `graft-inject` CLI that auto-wires your kernel, a `vesl-test` harness, and a full walkthrough — none of which you need to clone this monorepo for.

For developers integrating by hand or from Docker, [GRAFTING.md](templates/GRAFTING.md) is the long-form guide.

### Manual Setup

For contributors who want a local nockchain checkout and bare-metal builds.

Prerequisites: [nockchain](https://github.com/zorp-corp/nockchain) monorepo cloned and built at a sibling path, with `hoonc` and `nockchain` in your PATH. Rust nightly `2025-11-26` (pinned in `hull/rust-toolchain`).

```bash
git clone https://github.com/zkVesl/vesl.git
cd vesl
cp vesl.toml.example vesl.toml     # edit nock_home if your layout differs
make setup                          # create hoon symlinks
make build                          # compile hull
make demo-local                     # run the pipeline (no chain needed)
```

Run `make help` for all available targets. Configuration lives in `vesl.toml` — see `vesl.toml.example` for options. Environment variables (`NOCK_HOME`, `OLLAMA_URL`, `API_PORT`) override config file values.

After running the demo, `make inspect` shows what settled — current root, note count, recent note summaries. Requires a running hull (`--serve` mode).


## Test

```bash
make test-unit                      # 99 unit tests
make test                           # all tests (unit + e2e)
```

Fakenet (live local chain):

```bash
./scripts/fakenet-harness.sh run    # boot nodes, run 20 integration tests, tear down
```

Hoon tests are compile-time assertions — build success means pass:

```bash
hoonc --new protocol/tests/red-team.hoon hoon/
hoonc --new protocol/tests/prove-verify.hoon hoon/
```


## Settlement Modes

Vesl supports three settlement modes. Set via `--settlement-mode`, `VESL_SETTLEMENT_MODE`, or `settlement_mode` in `vesl.toml`.

| Mode | What happens | Chain required |
|------|-------------|----------------|
| `local` | Kernel verifies, no chain interaction. Default. | No |
| `fakenet` | Full pipeline — sign, build tx, submit to a local nockchain fakenet. | Yes (local) |
| `dumbnet` | Same as fakenet but uses a real seed phrase for key derivation. | Yes (live) |

Precedence: CLI flag > environment variable > `vesl.toml` > mode defaults. Passing `--chain-endpoint` or `--submit` without an explicit mode infers `fakenet`.


## Fakenet Settlement Walkthrough

Run the full pipeline: ingest documents, retrieve against a query, verify in the Hoon kernel, build a settlement transaction, sign it, and submit to a local chain.

```bash
# 1. Build everything
make setup                              # hoon symlinks
make build                              # compile hull (release)

# 2. Boot a local fakenet (hub + miner, background)
./scripts/fakenet-harness.sh start

# 3. Run the demo with live settlement
./scripts/demo.sh --fakenet

# 4. Or drive it manually via the HTTP API
cd hull && cargo run -- --new --serve --settlement-mode fakenet

# In another terminal:
curl -X POST http://127.0.0.1:3000/ingest \
  -H 'Content-Type: application/json' \
  -d '{"documents": ["Q3 revenue: $47M, up 12% YoY"]}'

curl -X POST http://127.0.0.1:3000/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "Summarize Q3 financial position", "top_k": 2}'

# /query triggers: retrieve → LLM → manifest → kernel verify → sign → settle
# The response includes the settlement result and transaction ID.

# 5. Run the full E2E test suite against the running fakenet
./scripts/fakenet-harness.sh test

# 6. Tear down
./scripts/fakenet-harness.sh stop
```

Or do it all in one shot:

```bash
./scripts/fakenet-harness.sh run        # boot → test → teardown
```

The harness mines to a demo signing key so the hull can spend coinbase UTXOs without wallet setup.


## Standalone Crates

These work independently of Vesl. Any NockApp can use them. Built on primitives from the [nockchain](https://github.com/zorp-corp/nockchain) monorepo — packaged as standalone libraries with documentation.

**[nock-noun-rs](crates/nock-noun-rs/)** — Build Nock nouns from Rust without reading 57K lines of wallet code. NockStack helpers, cord/tag/loobean builders, jam/cue round-trips. Handles the footguns (loobeans are inverted, cords aren't strings, lists are null-terminated) so you don't have to.

**[nockchain-tip5-rs](crates/nockchain-tip5-rs/)** — Standalone tip5 Merkle tree. ~100 arithmetic constraints per hash vs ~30,000 for SHA-256 — 100x cheaper in ZK circuits. Cross-runtime aligned: Rust output is byte-identical to Hoon.

**[vesl-test](protocol/lib/vesl-test.hoon)** — Compile-time Hoon testing. Eight assertion arms, zero configuration, no test runner. If it builds, it passes.


## Compile the Kernel

```bash
hoonc --new protocol/lib/vesl-kernel.hoon hoon/
cp out.jam assets/vesl.jam
```

Use `--new` after modifying Hoon source. hoonc caches aggressively.


## HTTP API

```bash
cd hull && cargo run -- --new --serve
```

| Endpoint | Method | |
|----------|--------|-|
| `/ingest` | POST | documents in, Merkle tree out |
| `/query` | POST | natural language query, triggers retrieval + settlement |
| `/prove` | POST | like `/query` but adds STARK proof (needs `--stack-size large`) |
| `/status` | GET | tree state, settled notes, root |
| `/health` | GET | liveness |

Use `--new` on first boot (or after kernel recompilation) to avoid stale NockApp state. For STARK proving, boot with `--stack-size huge` (see hardware requirements below). For real LLM inference, pass `--ollama-url http://localhost:11434`. Works with remote Ollama instances too, e.g. RunPod: `--ollama-url https://{pod-id}-11434.proxy.runpod.net`.

The server binds to `127.0.0.1` by default. To expose to the network, pass `--bind-addr 0.0.0.0`. For dumbnet mode, pass the signing key via `--seed-phrase-file <path>` (reads one line, trimmed) instead of `--seed-phrase` to keep the value out of `ps` output.


## Hardware Requirements

`/query` and `/settle` run on modest hardware (4 GB RAM, `--stack-size normal`).

`/prove` generates a STARK proof and needs significantly more. The Nockchain STARK prover allocates a 64 GB NockStack and is CPU-bound during FRI commitment and constraint evaluation.

| | Verify only | STARK proof |
|-|-------------|-------------|
| RAM | 4 GB | 64+ GB |
| Stack flag | `--stack-size normal` | `--stack-size huge` |

On Linux, enable overcommit for the large virtual allocation:

```bash
sudo sysctl -w vm.overcommit_memory=1
```


## License

[MIT](LICENSE)

~
