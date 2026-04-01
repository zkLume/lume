# Vesl

Verified RAG on Nockchain.

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
  tests/                    13 compile-time assertion tests

hull/                   Rust — the off-chain pipeline
  src/merkle.rs             tip5 Merkle tree (cross-runtime aligned with Hoon)
  src/chain.rs              on-chain settlement + confirmation
  src/api.rs                HTTP: /ingest, /query, /prove, /status
  src/tx_builder.rs         settlement transaction construction
  src/signing.rs            Schnorr signing
  src/ingest.rs             document chunking
  src/llm.rs                LLM integration (Ollama, trait-based)
  tests/                    pipeline, adversarial, fakenet (20 E2E tests)

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
make test-unit                      # 88 unit tests
make test                           # all tests (unit + e2e)
```

Fakenet (live local chain):

```bash
./scripts/fakenet-harness.sh run    # boot nodes, run 20 tests, tear down
```

Hoon tests are compile-time assertions — build success means pass:

```bash
hoonc --arbitrary protocol/tests/red-team.hoon hoon/
hoonc --arbitrary protocol/tests/prove-verify.hoon hoon/
```


## Standalone Crates

These work independently of Vesl. Any NockApp can use them.

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


## License

[MIT](LICENSE)

~
