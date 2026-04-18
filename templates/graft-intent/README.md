# graft-intent

A NockApp with a custom (non-RAG) verification gate grafted in.

## Why This Exists

`graft-mint` and `graft-settle` use RAG verification — manifests, Merkle proofs, prompt reconstruction. That's one gate. The Graft doesn't care what your gate does. This template proves it.

The verification gate here is one line:

```hoon
=((hash-leaf ;;(@ data)) expected-root)
```

Hash the data, compare to root. No manifest types, no `sur/vesl.hoon`, no `rag-logic.hoon`. The Graft is domain-agnostic — the gate is yours.

## What's Grafted

**Domain logic:**
- `%declare intent` — register an intent string
- `/intent/<id>` — peek at an intent
- `/count` — how many intents

**Grafted verification (custom gate):**
- `%settle-register hull root` — register a Merkle root
- `%settle-verify payload` — verify data against root via hash gate
- `%settle-note payload` — verify + settle (state transition + replay guard)
- `/settle-registered/<hull>`, `/settle-root/<hull>`, `/settle-noted/<note-id>`

## The Custom Gate Pattern

Define your gate inline where you delegate pokes:

```hoon
=/  hash-gate=verify-gate
  |=  [note-id=@ data=* expected-root=@]
  ^-  ?
  =((hash-leaf ;;(@ data)) expected-root)
=/  [efx=(list settle-effect) new-settle=settle-state]
  (settle-poke settle.state lc hash-gate)
```

The gate signature is `$-([note-id=@ data=* expected-root=@] ?)`. Cast `data` to your domain type, verify however you want, return a loobean. Bind `note-id` into the data if you want pre-commit protection (see `.dev/AUDIT_REPORT.md` H-03).

## Build & Run

```bash
# Compile Hoon kernel (requires $NOCK_HOME for tip5 primitives)
hoonc hoon/app/app.hoon $NOCK_HOME/hoon/

# Build Rust binary
cargo build

# Run
cargo run
```

## Files

```
hoon/
  app/app.hoon          — the kernel (intents + custom hash gate)
  lib/settle-graft.hoon — composable state and poke dispatcher
  lib/vesl-merkle.hoon  — Merkle primitives (tip5)
  common/wrapper.hoon   — NockApp protocol
src/main.rs             — Rust driver with Mint commitment demo
```

## Writing Your Own Gate

The gate type is `verify-gate`:

```hoon
+$  verify-gate  $-([note-id=@ data=* expected-root=@] ?)
```

`data` is opaque `*`. Cast it to whatever your domain needs:

```hoon
::  hash comparison (this template)
=((hash-leaf ;;(@ data)) expected-root)

::  RAG manifest verification (graft-mint, graft-settle)
(verify-manifest ;;(manifest data) expected-root)

::  signature check (your domain)
(verify-signature ;;(signed-payload data) expected-root)

::  always true (testing)
%.y
```

~
