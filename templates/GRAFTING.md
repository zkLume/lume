# How to Graft Mint onto Your NockApp

You have a NockApp. It does something useful. Now you want tamper-evident data commitment — Merkle roots, inclusion proofs, the works. You don't want to write hash functions or proof verification logic.

The Graft pattern attaches Vesl's verification infrastructure to your kernel as a composable library. Ten minutes. Three lines of poke delegation. No verification code written.

## What You Get

| Tier | Capability |
|------|-----------|
| **Mint** (Rust) | Build Merkle trees, generate proofs, get roots |
| **Guard** (Rust) | Verify proofs against roots locally |
| **Graft** (Hoon) | Register roots, verify manifests, settle notes — in-kernel |

Mint and Guard are pure math. No kernel boot required. The Graft adds state tracking and guard logic to your Hoon kernel.

## Step 1: Add the Hoon Files

Copy these into your template's `hoon/` directory:

```
hoon/
  lib/vesl-graft.hoon    # state + poke dispatcher (gate-agnostic)
  lib/vesl-merkle.hoon   # Merkle primitives (tip5)
```

For RAG verification, also copy:

```
hoon/
  sur/vesl.hoon          # RAG type definitions (manifest, chunk, etc.)
  lib/rag-logic.hoon    # RAG verification gates (verify-manifest)
```

These live in `protocol/sur/` and `protocol/lib/` in the Vesl repo. For non-RAG gates, only `vesl-graft.hoon` and `vesl-merkle.hoon` are required — see [Custom Gates](#custom-gates--beyond-rag) below.

## Step 2: Import the Graft

At the top of your kernel (`hoon/app/app.hoon`):

```hoon
/-  *vesl             :: RAG types (only needed for RAG gates)
/+  *vesl-graft       :: state + poke dispatcher
/+  *rag-logic       :: RAG verification gates (only for RAG)
/=  *  /common/wrapper
```

## Step 3: Compose State

Add `vesl-state` to your `versioned-state`. It tracks which roots are registered and which notes are settled:

```hoon
+$  versioned-state
  $:  %v1
      vesl=vesl-state          ::  [registered=(map @ @) settled=(set @)]
      ::  ...your state fields below...
      items=(map @t @)
  ==
```

## Step 4: Include Graft Causes

Add `vesl-cause` to your cause union. It brings `%vesl-register`, `%vesl-verify`, and `%vesl-settle`:

```hoon
+$  cause
  $%  [%add-item key=@t val=@]    ::  your domain poke
      vesl-cause                   ::  brings all %vesl-* pokes
  ==
```

## Step 5: Delegate Pokes

In your `++poke` arm, delegate Vesl causes to `vesl-poke`. Define your verification gate and pass it as the third argument:

```hoon
  %vesl-register
=/  lc=vesl-cause  [%vesl-register hull.u.act root.u.act]
=/  rag-gate=verify-gate
  |=  [data=* expected-root=@]
  ^-  ?
  =/  mani  ;;(manifest data)
  (verify-manifest mani expected-root)
=/  [efx=(list vesl-effect) new-vesl=vesl-state]
  (vesl-poke vesl.state lc rag-gate)
:_  state(vesl new-vesl)
^-  (list effect)  efx
```

Same pattern for `%vesl-verify` and `%vesl-settle`. Copy-paste, change the cause tag. The gate can be any function matching `$-([data=* expected-root=@] ?)` — see [Custom Gates](#custom-gates--beyond-rag).

## Step 6: Delegate Peeks

In your `++peek` arm, fall through to `vesl-peek` for unrecognized paths:

```hoon
++  peek
  |=  =path
  ^-  (unit (unit *))
  ?+  path  (vesl-peek vesl.state path)    ::  fallthrough
    [%item key=@t ~]  ...your peeks...
  ==
```

This gives you `/registered/<hull>`, `/settled/<note-id>`, and `/root/<hull>` for free.

## Step 7: Rust Side — Add Dependencies

In your `Cargo.toml`:

```toml
vesl-core = { path = "../../crates/vesl-core" }
nock-noun-rs = { path = "../../crates/nock-noun-rs" }
```

## Step 8: Commit Data with Mint

```rust
use vesl_core::Mint;

let mut mint = Mint::new();
let leaves: Vec<&[u8]> = documents.iter()
    .map(|d| d.as_bytes())
    .collect();
mint.commit(&leaves);

let root = mint.root().expect("committed");
```

## Step 9: Register the Root

Build a `%vesl-register` poke and send it to the kernel:

```rust
use nock_noun_rs::make_tag_in;
use nockapp::noun::slab::NounSlab;
use nockvm::noun::{D, T};

let mut slab = NounSlab::new();
let tag = make_tag_in(&mut slab, "vesl-register");
let poke = T(&mut slab, &[tag, D(hull_id), D(root_atom)]);
slab.set_root(poke);

app.poke(SystemWire.to_wire(), slab).await?;
```

Note: `make_tag_in` handles tags longer than 8 bytes (like `vesl-register`) that don't fit in a u64 direct atom. Use it instead of `D(tas!(b"..."))` for long tags.

## Step 10: Verify Proofs with Guard

```rust
use vesl_core::Guard;

let mut guard = Guard::new();
guard.register_root(root).unwrap();

for (i, doc) in documents.iter().enumerate() {
    let proof = mint.proof(i).unwrap();
    let valid = guard.check(doc.as_bytes(), &proof, &root);
    // valid is true if the document is bound to the Merkle root
}
```

Guard verification is local — no kernel, no network, no async. Pure math.

## Compile

The kernel needs `$NOCK_HOME/hoon/` for tip5 primitives (zeke.hoon):

```bash
hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
```

Or use the pre-compiled `out.jam` from the graft-mint template (1.5MB).

## The Tiers

If you only need commitment: use Mint (Rust-only, no kernel).

If you need commitment + verification: add Guard (still Rust-only).

If you need in-kernel state tracking: add the Graft (Hoon library).

If you need settlement with replay protection: delegate `%vesl-settle` (Settle pattern).

| Need | Use | Kernel? |
|------|-----|---------|
| Hash data, get roots | Mint | No |
| Verify proofs | Mint + Guard | No |
| Register roots in kernel | Mint + Graft | Yes |
| Verify in kernel | Graft (%vesl-verify) | Yes |
| Settle notes | Graft (%vesl-settle) | Yes |
| STARK proofs | Full vesl-kernel + prover | Yes (18MB) |

## Custom Gates — Beyond RAG

The Graft is domain-agnostic. The examples above use a RAG verification gate (cast data to a manifest, verify Merkle proofs, check prompt reconstruction). That's one gate. The `verify-gate` type is:

```hoon
+$  verify-gate  $-([data=* expected-root=@] ?)
```

`data` is opaque `*`. Cast it to your domain type and return a loobean. Some examples:

```hoon
::  RAG manifest verification (graft-mint, graft-settle)
|=  [data=* expected-root=@]
=/  mani  ;;(manifest data)
(verify-manifest mani expected-root)

::  Simple hash comparison (graft-intent)
|=  [data=* expected-root=@]
=((hash-leaf ;;(@ data)) expected-root)

::  Signature verification (your domain)
|=  [data=* expected-root=@]
=/  payload  ;;([sig=@ msg=@] data)
(verify-sig sig.payload msg.payload expected-root)

::  Always-true gate (testing)
|=  [data=* expected-root=@]
%.y
```

### How to Use a Custom Gate

1. **Import what you need.** For hash-based gates: `/+  *vesl-merkle`. For RAG: `/+  *rag-logic` and `/-  *vesl`. For your own logic: import your own library.

2. **Define the gate inline** in your poke delegation:

```hoon
  %vesl-settle
=/  lc=vesl-cause  [%vesl-settle payload.u.act]
=/  my-gate=verify-gate
  |=  [data=* expected-root=@]
  ^-  ?
  :: ...your verification logic...
  %.y
=/  [efx=(list vesl-effect) new-vesl=vesl-state]
  (vesl-poke vesl.state lc my-gate)
:_  state(vesl new-vesl)
^-  (list effect)  efx
```

3. **Build the payload from Rust.** The Graft expects a jammed `graft-payload`:

```hoon
+$  graft-payload
  $:  note=[id=@ hull=@ root=@ state=[%pending ~]]
      data=*
      expected-root=@
  ==
```

`data` is whatever your gate expects. JAM the whole payload, pass it as the `payload` field in `%vesl-settle` or `%vesl-verify`.

### The graft-intent Template

[`graft-intent`](./graft-intent/) is a working example of a non-RAG gate. No `sur/vesl.hoon`, no `rag-logic.hoon`. The gate is one line: hash the data, compare to root. Read it to see the pattern stripped to the minimum.

## Reference Templates

- [`graft-mint`](./graft-mint/) — Mint + Guard with RAG verification gate
- [`graft-settle`](./graft-settle/) — Full settlement lifecycle with RAG gate
- [`graft-intent`](./graft-intent/) — Custom hash gate, no RAG types

~
