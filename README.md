# Vesl

Verifiable RAG on Nockchain.

Ingest data into a tip5 Merkle tree. Retrieve chunks with inclusion proofs. Verify prompt integrity in a Hoon kernel. Settle on-chain. The root is the only thing that touches the chain.


## Demo

```bash
# Local pipeline — no chain required, runs in ~30 seconds
./scripts/demo.sh --no-chain

# Full pipeline with live fakenet settlement
./scripts/demo.sh
```

The demo ingests documents, retrieves against a query, verifies in the Hoon kernel, and settles. `--no-chain` skips chain interaction so you can see the pipeline without booting a fakenet.


## Structure

```
protocol/                 Hoon — the trust anchor
  sur/vesl.hoon             types
  lib/vesl-logic.hoon       verification gates (tip5 Merkle, prompt integrity)
  lib/vesl-kernel.hoon      NockApp kernel (poke/peek/load)
  lib/vesl-prover.hoon      STARK proof generation
  lib/vesl-verifier.hoon    STARK proof verification
  tests/                    14 compile-time assertion tests

hull/                   Rust — the off-chain pipeline
  src/merkle.rs             tip5 Merkle tree (cross-runtime aligned with Hoon)
  src/chain.rs              on-chain settlement + confirmation
  src/api.rs                HTTP: /ingest, /query, /prove, /status
  src/tx_builder.rs         settlement transaction construction
  src/signing.rs            Schnorr signing
  src/ingest.rs             document chunking
  src/llm.rs                LLM integration (Ollama, trait-based)
  tests/                    pipeline, adversarial, prover, fakenet (37 E2E tests)

crates/                   Standalone crates (usable without Vesl)
  nock-noun-rs/             Nock noun construction from Rust
  nockchain-tip5-rs/        tip5 Merkle tree + hash functions

kernels/vesl/             kernel compilation crate
assets/vesl.jam           compiled kernel
demo/docs/                sample documents for the demo pipeline
scripts/                  demo + fakenet harness
hoon/                     symlink tree (setup-hoon-tree.sh creates links to $NOCK_HOME)
```


## Quick Start

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

Use `--new` on first boot (or after kernel recompilation) to avoid stale NockApp state. For STARK proving, boot with `--stack-size large`. For real LLM inference, pass `--ollama-url http://localhost:11434`. Works with remote Ollama instances too, e.g. RunPod: `--ollama-url https://{pod-id}-11434.proxy.runpod.net`.

The server binds to `127.0.0.1` by default. To expose to the network, pass `--bind-addr 0.0.0.0`. For dumbnet mode, pass the signing key via `--seed-phrase-file <path>` (reads one line, trimmed) instead of `--seed-phrase` to keep the value out of `ps` output.


## License

[MIT](LICENSE)

~
