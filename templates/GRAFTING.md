# How to Graft Ink onto Your NockApp

You have a NockApp. It does something useful. Now you want tamper-evident data commitment — Merkle roots, inclusion proofs, the works. You don't want to write hash functions or proof verification logic.

The Graft pattern attaches Vesl's verification infrastructure to your kernel as a composable library. Ten minutes. Three lines of poke delegation. No verification code written.

## What You Get

| Tentacle | Capability |
|----------|-----------|
| **Ink** (Rust) | Build Merkle trees, generate proofs, get roots |
| **Grip** (Rust) | Verify proofs against roots locally |
| **Graft** (Hoon) | Register roots, verify manifests, settle notes — in-kernel |

Ink and Grip are pure math. No kernel boot required. The Graft adds state tracking and guard logic to your Hoon kernel.

## Step 1: Add the Hoon Files

Copy these into your template's `hoon/` directory:

```
hoon/
  sur/vesl.hoon          # type definitions
  lib/vesl-graft.hoon    # state + poke dispatcher
  lib/vesl-logic.hoon    # verification gates (graft dependency)
```

These live in `protocol/sur/` and `protocol/lib/` in the Vesl repo.

## Step 2: Import the Graft

At the top of your kernel (`hoon/app/app.hoon`):

```hoon
/-  *vesl
/+  *vesl-graft
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

In your `++poke` arm, delegate Vesl causes to `vesl-poke`. Each one is three lines:

```hoon
  %vesl-register
=/  lc=vesl-cause  [%vesl-register hull.u.act root.u.act]
=/  [efx=(list vesl-effect) new-vesl=vesl-state]
  (vesl-poke vesl.state lc)
:_  state(vesl new-vesl)
^-  (list effect)  efx
```

Same pattern for `%vesl-verify` and `%vesl-settle`. Copy-paste, change the cause tag.

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
vesl-mantle = { path = "../../crates/vesl-mantle" }
nock-noun-rs = { path = "../../crates/nock-noun-rs" }
```

## Step 8: Commit Data with Ink

```rust
use vesl_mantle::Ink;

let mut ink = Ink::new();
let leaves: Vec<&[u8]> = documents.iter()
    .map(|d| d.as_bytes())
    .collect();
ink.commit(&leaves);

let root = ink.root().expect("committed");
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

## Step 10: Verify Proofs with Grip

```rust
use vesl_mantle::Grip;

let mut grip = Grip::new();
grip.register_root(root);

for (i, doc) in documents.iter().enumerate() {
    let proof = ink.proof(i);
    let valid = grip.check(doc.as_bytes(), &proof, &root);
    // valid is true if the document is bound to the Merkle root
}
```

Grip verification is local — no kernel, no network, no async. Pure math.

## Compile

The kernel needs `$NOCK_HOME/hoon/` for tip5 primitives (zeke.hoon):

```bash
hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
```

Or use the pre-compiled `out.jam` from the graft-ink template (1.5MB).

## The Weight Classes

If you only need commitment: use Ink (Rust-only, no kernel).

If you need commitment + verification: add Grip (still Rust-only).

If you need in-kernel state tracking: add the Graft (Hoon library).

If you need settlement with replay protection: delegate `%vesl-settle` (Beak pattern).

| Need | Use | Kernel? |
|------|-----|---------|
| Hash data, get roots | Ink | No |
| Verify proofs | Ink + Grip | No |
| Register roots in kernel | Ink + Graft | Yes |
| Verify in kernel | Graft (%vesl-verify) | Yes |
| Settle notes | Graft (%vesl-settle) | Yes |
| STARK proofs | Full vesl-kernel + prover | Yes (18MB) |

## Reference Templates

- [`graft-ink`](./graft-ink/) — Complete example with Ink + Grip
- [`graft-beak`](./graft-beak/) — Extends graft-ink with settlement

~
