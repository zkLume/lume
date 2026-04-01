# Vesl

Vesl is a verified execution and settlement layer on Nockchain. It ingests data into a Merkle tree, retrieves chunks against queries with inclusion proofs, verifies prompt integrity in a Hoon kernel, and generates STARK proofs that the Nock VM executed the settlement correctly. The root is the only thing that touches the chain.


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

kernels/vesl/             kernel compilation crate
assets/vesl.jam           compiled kernel
scripts/                  demo + fakenet harness
hoon/                     symlink tree (setup-hoon-tree.sh creates links to $NOCK_HOME)
```

## Build

Requires the [nockchain](https://github.com/zorp-corp/nockchain) monorepo cloned and built at a sibling path (`../../nockchain` from `hull/`), with `hoonc` and `nockchain` in your PATH.

```bash
git clone https://github.com/zkVesl/vesl.git
cd vesl

# Point at the nockchain monorepo and create hoon/ symlinks
export NOCK_HOME=~/path/to/nockchain
./scripts/setup-hoon-tree.sh

cd hull && cargo build --release
```

Rust toolchain: `nightly-2025-11-26` (pinned in `hull/rust-toolchain`). The Cargo dependencies expect the nockchain monorepo at `../../nockchain` relative to `hull/` — adjust paths in `hull/Cargo.toml` if your layout differs.

## Test

```bash
cargo test --lib                    # 88 unit tests
cargo test --test e2e_pipeline      # kernel boot + settlement
cargo test --test e2e_adversarial   # 12 adversarial tests (kernel + HTTP)
cargo test                          # all of the above
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

## Compile the kernel

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
