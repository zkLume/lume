# graft-sigil

A NockApp with Vesl's Sigil + Vigil tiers grafted in.

## Why This Exists

You have a NockApp. You want to add Merkle commitment and verification to it. You don't want to write any verification logic. The Graft pattern lets you compose Vesl's verification state and poke handlers into your kernel alongside your domain logic.

This template is the reference implementation of a 10-minute Graft.

## What's Grafted

The kernel has two layers:

**Domain logic** (yours):
- `%put key val` — store a note
- `%del key` — delete a note
- `/note/<key>` — peek at a note
- `/count` — how many notes

**Grafted verification** (Vesl's):
- `%vesl-register hull root` — register a Merkle root
- `%vesl-verify payload` — verify a manifest against a registered root
- `%vesl-settle payload` — verify + settle a note
- `/registered/<hull>` — is this hull registered?
- `/root/<hull>` — what root did this hull register?

Zero verification code in the kernel. The `++poke` arm delegates `%vesl-*` causes to `vesl-poke` from `vesl-graft.hoon`. Three lines per cause.

## The Pattern

In your kernel's state, compose `vesl-state`:

```hoon
+$  versioned-state
  $:  %v1
      vesl=vesl-state          :: grafted
      notes=(map @t @t)        :: yours
  ==
```

In your poke arm, delegate:

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

In your peek arm, fall through:

```hoon
?+  path  (vesl-peek vesl.state path)
  [%note key=@t ~]  ...your peeks...
==
```

That's the Graft. Your domain logic stays clean. Vesl verification is composable infrastructure.

## Rust Side

The Rust driver demonstrates the Sigil + Vigil workflow:

1. **Sigil** builds a Merkle tree from your data and gives you a root + proofs
2. You poke `%vesl-register` to tell the kernel about the root
3. **Vigil** verifies individual proofs against the root (local, no kernel needed)

## Build & Run

```bash
# Compile Hoon kernel (requires $NOCK_HOME for tip5 primitives)
hoonc hoon/app/app.hoon $NOCK_HOME/hoon/

# Or use the pre-compiled out.jam (already included)

# Build Rust binary
cargo build

# Run
cargo run
```

## Files

```
hoon/
  app/app.hoon          — the kernel (domain + graft)
  lib/vesl-graft.hoon   — composable state and poke dispatcher
  lib/vesl-logic.hoon   — RAG verification gates
  lib/vesl-merkle.hoon  — Merkle primitives (tip5)
  sur/vesl.hoon          — type definitions
  common/wrapper.hoon  — NockApp protocol
src/main.rs            — Rust driver with Sigil + Vigil demo
```

## What to Read

Start with `hoon/app/app.hoon`. The Graft delegation is at the bottom of the `++poke` arm — look for the "grafted verification" comment. Then compare the Rust side (`src/main.rs`) to see how `Sigil::commit()` and `Vigil::check()` mirror the kernel's registration and verification.

~
