# graft-scaffold

Starter template for building a grafted NockApp with Vesl. All Hoon dependencies are bundled — no `$NOCK_HOME` required.

## What's included

- Full Vesl graft wiring (register, verify, settle)
- Default hash-comparison verification gate
- One placeholder domain poke (`%my-action`) to rename
- Rust driver demonstrating the complete lifecycle

## Quick start

1. Copy this directory
2. Compile the kernel:

```bash
hoonc --new hoon/app/app.hoon hoon/
```

3. Build and run:

```bash
cargo +nightly build
cargo +nightly run
```

## Customize

The Hoon kernel (`hoon/app/app.hoon`) has `CUSTOMIZE` markers:

- **`%my-action`** — rename to your domain poke tag
- **`items`** — replace with your state fields
- **`versioned-state`** — add fields after `vesl=vesl-state`
- **`++peek`** — add your query paths

The verification gate defaults to `=((hash-leaf ;;(@ data)) expected-root)`. Replace with your domain logic (manifest verification, signature check, etc.).

## File tree

```
hoon/
  app/app.hoon              main kernel (graft pre-wired)
  lib/vesl-graft.hoon       state + poke dispatcher
  lib/vesl-merkle.hoon      Merkle primitives (tip5)
  common/wrapper.hoon       state versioning
  common/zeke.hoon          tip5 hash chain
  common/ztd/               tip5 math tables (8 files)
src/main.rs                 Rust driver (full lifecycle)
Cargo.toml                  dependencies (local paths)
```

## Dependencies

Adjust the paths in `Cargo.toml` to point to your local clones of nockchain and vesl.

Requires nightly Rust (`cargo +nightly build`).

## Using nockup?

See [vesl-nockup](https://github.com/zkVesl/vesl-nockup) for a nockup-packaged version: pre-resolved git deps (no sibling repos required), a `graft-inject` CLI that auto-wires your kernel, and a `vesl-test` harness with a standard lifecycle suite.

~
