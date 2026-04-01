# Architecture

## Overview

Vesl is a verifiable RAG (Retrieval-Augmented Generation) pipeline built on Nockchain. It lets an AI agent answer questions using private documents while generating a mathematical proof that the answer was derived honestly — the right documents were retrieved, the prompt wasn't tampered with, and the output came from the claimed computation.

The proof settles on Nockchain as a UTXO Note. Anyone can verify the proof without seeing the private data.

### nockvm: The Embedded Nock Interpreter

Throughout this document (and the codebase), "nockvm" refers to the Nock bytecode interpreter embedded inside the Rust binary. It is not a separate virtual machine or a second process — it's a deterministic interpreter that runs compiled Hoon (as Nock bytecode, serialized via JAM) within the same Rust process that runs the rest of the pipeline.

The actual runtime topology is one binary with two runtimes:

```
Rust process
├── native Rust code (tip5, Merkle, noun building, HTTP API, chain client)
└── nockvm interpreter
    └── vesl.jam kernel (compiled Hoon → Nock bytecode)
```

Rust interacts with the kernel only through pokes (JAM'd nouns in) and peeks (nouns out). The kernel's execution is hermetic — Rust cannot reach into the interpreter's state or override its crash semantics. This narrow interface is what makes the kernel a trust anchor: it either accepts a payload and settles, or it crashes, and there's no way to fudge the result from the Rust side.

When the docs say "cross-runtime alignment," they mean the invariant that native Rust computation and interpreted Nock computation must produce byte-identical results for the same inputs.

## The Hull Architecture

The system has four tiers:

```
Tier 1: Data Commitment        Tier 2: Local Inference
  Documents → Chunks              Query → Retrieve Top-K
  Chunks → Merkle Tree            Chunks + Query → Prompt
  Root → On-Chain Note            Prompt → LLM → Output
                                  Output → Inference Manifest

Tier 3: Nock-Prover             Tier 4: Settlement
  Manifest → Verify Chunks       Verified Note → STARK Proof
  Verify Prompt Integrity         Proof → Nockchain Note
  Verify Merkle Proofs            Note → On-Chain Settlement
  State: pending → settled
```

### Tier 1: Data Commitment

Documents are split into chunks, each hashed with tip5 (a STARK-native algebraic hash). The chunks form a Merkle tree. Only the root is committed on-chain — the documents stay local.

### Tier 2: Local Inference

Given a query, the hull retrieves the most relevant chunks, constructs a deterministic prompt (`query + \n + chunk0 + \n + chunk1 + ...`), sends it to a local LLM (Ollama), and packages the result as an Inference Manifest containing the query, retrieved chunks with Merkle proofs, the exact prompt sent, and the LLM's output.

### Tier 3: Nock-Prover (The Kernel)

The Hoon kernel (`vesl-kernel.hoon`) is a NockApp that verifies the manifest:

1. **Merkle verification** — each retrieved chunk's proof is validated against the committed root
2. **Prompt integrity** — the prompt is reconstructed from the query + chunks and compared byte-for-byte
3. **Injection detection** — the prompt must contain only the query and retrieved chunk data, nothing else
4. **State transition** — the note moves from `%pending` to `%settled`

If any check fails, the kernel crashes (`?>` assertion). The crash is the security guarantee — a valid STARK proof of the settlement computation is proof that all checks passed.

### Tier 4: Settlement

The settled note is encoded as NoteData entries in a standard Nockchain Note (Strategy A: data-in-note). The STARK proof attests that the Nock VM correctly executed the settlement logic.

## Component Map

```
protocol/                        Hoon protocol layer (trust anchor)
  sur/vesl.hoon                  Type definitions (chunk, manifest, note, etc.)
  lib/vesl-logic.hoon            Verification gates (hash, verify, settle)
  lib/vesl-kernel.hoon           NockApp kernel (load/peek/poke lifecycle)
  lib/vesl-entrypoint.hoon       ABI boundary (jam/cue serialization)
  lib/vesl-prover.hoon           STARK proof generation (arbitrary Nock)
  lib/vesl-verifier.hoon         STARK proof verification wrapper
  lib/vesl-stark-verifier.hoon   STARK verifier fork (non-puzzle proofs)
  tests/                         8 test files, compile-time assertions

hull/                          Rust hull (off-chain pipeline)
  src/merkle.rs                  tip5 Merkle tree (cross-runtime aligned)
  src/noun_builder.rs            Nock noun serialization for kernel pokes
  src/chain.rs                   On-chain settlement (NoteData encoding, gRPC)
  src/api.rs                     HTTP API (axum): /ingest, /query, /status
  src/ingest.rs                  Document chunking and tree building
  src/llm.rs                     LLM integration (Ollama, pluggable trait)
  src/retrieve.rs                Retrieval (keyword, pluggable trait)
  src/wallet.rs                  Wallet noun builders
  src/wallet_kernel.rs           In-process wallet kernel integration

kernels/vesl/                    Kernel compilation crate (embeds vesl.jam)
assets/vesl.jam                  Compiled Hoon kernel (~18MB)
scripts/                         Fakenet harness and environment config
```

## Hash Function: tip5

Vesl uses tip5, the same algebraic hash used in Nockchain's block validation. tip5 operates over the Goldilocks field (p = 2^64 - 2^32 + 1) and costs ~300 R1CS constraints per hash — 100x cheaper than SHA-256 in STARK circuits.

Leaf data is encoded via 7-byte little-endian chunking (each chunk guaranteed < 2^56 < p), fed to tip5's variable-length sponge. Pair hashing uses the fixed-rate 10-element sponge. Both Hoon and Rust implement identical encoding, verified by cross-runtime alignment tests.

## STARK Proofs

The STARK prover (`vesl-prover.hoon`) is a fork of Nockchain's `nock-prover.hoon` that accepts arbitrary `[subject formula]` pairs instead of PoW puzzles. The constraint system (compute-table, memory-table) enforces correct Nock VM execution regardless of which computation is proved.

The verifier (`vesl-stark-verifier.hoon`) is a minimal fork of `stark/verifier.hoon` — 8 lines changed to accept `[s f]` as parameters instead of deriving them from puzzle data. All FRI, linking-checks, and constraint polynomial evaluation are unchanged.

## Security Model

Six layers of defense, each independently verified by the adversarial test suite:

| Layer | What It Catches |
|-------|----------------|
| Mold boundary (`;;`) | Structural errors (malformed payloads) |
| Merkle verification | Tampered chunks, swapped proof paths |
| Prompt reconstruction | Injection attacks (appended instructions) |
| `settle-note` crash (`?>` / `!!`) | Any verification failure crashes the kernel |
| Registration guard | Settlement against uncommitted roots |
| Replay guard | Duplicate settlement attempts |

## Test Coverage

| Suite | Tests | What It Covers |
|-------|-------|----------------|
| Hoon compile-time | 8 | Merkle proofs, manifests, settlement, red-team (4 attack vectors), STARK prove+verify, cross-runtime, ABI |
| Rust unit | 79 | Merkle, noun builder, chain encoding, ingest, LLM, retrieval, wallet, HTTP API |
| E2E pipeline | 3 | Kernel boot, poke, settlement |
| E2E adversarial | 12 | 7 kernel attack vectors + 5 HTTP API vectors |
| E2E fakenet | 12 | Live chain connectivity, balance queries, full HTTP pipeline, wallet kernel |
