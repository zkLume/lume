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
  tests/                    8 compile-time assertion tests

hull/                   Rust — the off-chain pipeline
  src/merkle.rs             tip5 Merkle tree (cross-VM aligned with Hoon)
  src/chain.rs              on-chain settlement encoding (Strategy A)
  src/api.rs                HTTP: /ingest, /query, /status
  src/ingest.rs             document chunking
  src/llm.rs                LLM integration (Ollama, trait-based)
  tests/                    pipeline, adversarial, fakenet

kernels/vesl/             kernel compilation crate
assets/vesl.jam           compiled kernel
scripts/                  fakenet harness
```

## Build

Requires the [nockchain](https://github.com/zorp-corp/nockchain) monorepo cloned and built, with `hoonc` and `nockchain` in your PATH.

```bash
git clone https://github.com/sobchek/vesl.git
cd vesl

export NOCK_HOME=~/path/to/nockchain
./scripts/setup-hoon-tree.sh

cd hull && cargo build
```

Rust toolchain: `nightly-2025-11-26` (pinned in `hull/rust-toolchain`).

## Test

```bash
cargo test --lib                    # 79 unit tests
cargo test --test e2e_pipeline      # kernel boot + settlement
cargo test --test e2e_adversarial   # 7 kernel attack vectors, 5 HTTP vectors
cargo test                          # all of the above
```

Fakenet (live local chain):

```bash
./scripts/fakenet-harness.sh run    # boot nodes, run 12 tests, tear down
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
cd hull && cargo run
```

| Endpoint | Method | |
|----------|--------|-|
| `/ingest` | POST | documents in, Merkle tree out |
| `/query` | POST | natural language query, triggers retrieval + settlement |
| `/status` | GET | tree state, settled notes, root |
| `/health` | GET | liveness |

Expects Ollama at `localhost:11434` for inference.

## License

[MIT](LICENSE)
