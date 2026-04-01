# graft-beak

A NockApp with Vesl's full settlement tentacle grafted in.

## Why This Exists

`graft-ink` grafts commitment and verification. This template goes further: it grafts the **Beak** — full settlement lifecycle with replay protection. Notes transition from `%pending` to `%settled` and can never be settled twice.

Settlement is where soft state becomes hard record. The only hard part of an octopus.

## What's Grafted

**Domain logic:**
- `%submit title body` — submit a report (assigns incrementing ID)
- `/report/<id>` — peek at a report
- `/count` — how many reports

**Grafted tentacles (full settlement):**
- `%vesl-register hull root` — register Merkle root
- `%vesl-verify payload` — verify manifest (read-only)
- `%vesl-settle payload` — verify + settle note (state transition + replay guard)
- `/registered/<hull>`, `/root/<hull>`, `/settled/<note-id>`

The kernel's `%vesl-settle` handler:
1. Cues the jammed settlement-payload
2. Checks the root is registered (guard 1)
3. Checks the note isn't already settled (guard 2 — replay)
4. Verifies the full manifest against the root (guard 3)
5. Transitions the note to `%settled`

All five steps are handled by `vesl-poke` from `vesl-graft.hoon`. Your kernel just delegates.

## The Settlement Pattern

```
submit reports → commit to Merkle tree → register root
                                              ↓
              verify proofs ← Grip    settle notes ← Beak
                                              ↓
                                    permanent record
                                    (replay-protected)
```

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

## Upgrading from graft-ink

If you started with `graft-ink` and need settlement:

1. Add `%vesl-settle` to your cause type (it's already in `vesl-cause`)
2. Add the settle delegation in your poke arm (3 lines, same pattern)
3. Done. The Graft handles replay protection and state transitions.

~
