# graft-intent

A NockApp with a custom (non-RAG) verification gate grafted in.

## Why This Exists

`graft-sigil` and `graft-anchor` use RAG verification — manifests, Merkle proofs, prompt reconstruction. That's one gate. The Graft doesn't care what your gate does. This template proves it.

The verification gate here is one line:

```hoon
=((hash-leaf ;;(@ data)) expected-root)
```

Hash the data, compare to root. No manifest types, no `sur/vesl.hoon`, no `vesl-logic.hoon`. The Graft is domain-agnostic — the gate is yours.

## What's Grafted

**Domain logic:**
- `%declare intent` — register an intent string
- `/intent/<id>` — peek at an intent
- `/count` — how many intents

**Grafted verification (custom gate):**
- `%vesl-register hull root` — register a Merkle root
- `%vesl-verify payload` — verify data against root via hash gate
- `%vesl-settle payload` — verify + settle (state transition + replay guard)
- `/registered/<hull>`, `/root/<hull>`, `/settled/<note-id>`

## The Custom Gate Pattern

Define your gate inline where you delegate pokes:

```hoon
=/  hash-gate=verify-gate
  |=  [data=* expected-root=@]
  ^-  ?
  =((hash-leaf ;;(@ data)) expected-root)
=/  [efx=(list vesl-effect) new-vesl=vesl-state]
  (vesl-poke vesl.state lc hash-gate)
```

The gate signature is `$-([data=* expected-root=@] ?)`. Cast `data` to your domain type, verify however you want, return a loobean.

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
  lib/vesl-graft.hoon   — composable state and poke dispatcher
  lib/vesl-merkle.hoon  — Merkle primitives (tip5)
  common/wrapper.hoon   — NockApp protocol
src/main.rs             — Rust driver with Sigil commitment demo
```

## Writing Your Own Gate

The gate type is `verify-gate`:

```hoon
+$  verify-gate  $-([data=* expected-root=@] ?)
```

`data` is opaque `*`. Cast it to whatever your domain needs:

```hoon
::  hash comparison (this template)
=((hash-leaf ;;(@ data)) expected-root)

::  RAG manifest verification (graft-sigil, graft-anchor)
(verify-manifest ;;(manifest data) expected-root)

::  signature check (your domain)
(verify-signature ;;(signed-payload data) expected-root)

::  always true (testing)
%.y
```

~
