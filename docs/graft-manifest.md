# `graft.toml` schema

A graft manifest describes how `graft-inject` composes a Hoon library
into a host kernel's `app.hoon`. One manifest per graft, sibling to the
graft's `.hoon` file under `protocol/lib/` (or `hoon/lib/` after `sync.sh`
in vesl-nockup).

This document is the source of truth for the manifest format. The Rust
loader in `vesl-nockup/tools/graft-inject` implements it; graft authors
read this to write a manifest without reading the loader.

## Layout

```
protocol/lib/
  settle-graft.hoon       host library
  settle-graft.toml       manifest (this file's schema)
  mint-graft.hoon
  mint-graft.toml
  ...
```

Flat — no per-graft directory. The manifest's `name` field, not its
filename, is the canonical identifier the loader uses.

## `[graft]` — top-level metadata

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | string | yes | Canonical name. Matches the `--grafts <CSV>` argument. Must be unique across all manifests under the discovery root. |
| `version` | string | yes | Semver. Bumped when blocks change in a backwards-incompatible way. |
| `priority` | int | yes | Injection order. Lower = injected earlier. Lattice: 10–40 for commitment primitives, 50–99 for state-pattern grafts, 100+ for user/domain grafts. |
| `after` | string list | no | Soft ordering hints. Each entry names another graft that must inject earlier. Error at load time if an entry names a graft not present in the discovered set. Resolved after `priority` ties. |

Example:

```toml
[graft]
name     = "settle-graft"
version  = "0.1.0"
priority = 10
after    = []
```

## `[graft.blocks.*]` — injection blocks

A graft contributes one block per marker it claims. Five markers exist
in Stage 1: `imports`, `state`, `cause`, `poke`, `peek`. A block omitted
from the manifest is not injected for that marker — the marker is left
untouched (or, for `peek`, joins the chain only if other grafts contribute).

Each present block is a TOML sub-table with two fields:

| Field | Type | Required | Notes |
|---|---|---|---|
| `sentinel` | string | yes | Substring the loader scans for in the marker window to detect already-injected wiring. Per-graft. Used for idempotence. |
| `body` | string | yes | The Hoon to paste at the marker. Stored unindented; the loader re-applies indentation from the marker line's leading whitespace. Leading and trailing newlines on the string are trimmed before injection. |

### Sentinel rules

The sentinel must:
- Appear exactly once in the post-injection body for that marker.
- Be specific enough that no other graft's body or any user-written code
  in the marker window would match.
- Be short — the loader scans a fixed-size window after the marker line
  (10 lines for `imports`, 60 lines for `poke`, 20 lines for the rest);
  a long sentinel costs no more, but a short one is easier to read.

Conventional sentinels:
- `imports`: `*<graft-name>` (e.g., `*mint-graft`) — the import directive.
- `state`: `<field>=<type-name>` (e.g., `mint=mint-state`).
- `cause`: `<graft>-cause` (e.g., `mint-cause`) — the embedded cause union.
- `poke`: `%<graft>-<verb>` of the first arm (e.g., `%mint-commit`).
- `peek`: `<graft>-peek` (e.g., `mint-peek`) — the helper arm name.

### Body rules

- **Indentation**: each line in `body` is stored at the indentation it
  needs *relative to the marker*. The loader prepends the marker line's
  leading whitespace to every non-empty line. Empty lines stay empty.
- **Trim**: leading and trailing newlines on the `body` string are
  removed before composition. Use TOML's triple-quoted form for
  multi-line bodies; a leading newline after `"""` is convenient.
- **Two-space law**: every Hoon rune in the body must be followed by
  exactly two spaces (or end-of-line). The loader does not enforce this,
  but `hoonc` will fail downstream if violated.
- **Per-marker conventions**:
  - `imports`: one or more `/+` directives. No leading `::`.
  - `state`: a single `field=type` pair to splice into the kernel's
    `versioned-state $:` block.
  - `cause`: a single bare type name to splice into the `cause $%` union.
  - `poke`: arm bodies for the kernel's `?-` switch, with internal `::`
    separators between arms. Bodies start with `::` to separate from
    any pre-existing arm in the user's switch.
  - `peek`: a single Hoon expression that returns `(unit (unit *))`.
    Returns `~` for non-matching paths. The composer wraps each peek
    body into a chain — see Composition below.

## Composition

When multiple grafts contribute blocks for the same marker, `graft-inject`
composes them in `priority` order (lower first), `after`-hint order for
ties, then by `name`.

### Non-peek markers

Each graft's `body` is concatenated with a blank line between grafts.
The `poke` composer additionally guarantees that adjacent graft bodies
are separated by `::` — a graft body ending without `::` will get one
prepended to the next graft's body.

### `peek` marker

A peek chain. Each graft contributes two lines:

```hoon
=/  <name>-res  <peek.body>
?.  =(~ <name>-res)  <name>-res
```

The terminal `~` from the bare scaffold remains as the chain's final
fallback. A graft's peek body must return `~` (not `[~ ~]`) for paths
it doesn't handle, so the chain falls through to the next graft.

### Idempotence

- **Per-graft**: re-running `graft-inject` against an already-injected
  file scans each graft's marker window for that graft's `sentinel`. If
  found, the graft is skipped; other grafts may still inject.
- **Peek-chain**: the loader scans for existing `=/\s+(\S+)-res\s`
  bindings in the peek window and inserts new grafts' pairs immediately
  before the terminal `~`. A missing terminal `~` is a warning, not an
  error — the resulting chain may not parse, but `hoonc` will report it.

## Discovery and selection

`graft-inject` discovers manifests by scanning `--lib-dir` (default
`./hoon/lib/`) for files matching `*.toml` with a `[graft]` table. Files
without `[graft]` are ignored — TOML used for unrelated config can live
beside graft manifests without conflict.

CLI:

```
graft-inject [OPTIONS] [PATH]
  --grafts <CSV>    explicit grafts in injection order; bypasses auto-discover
  --exclude <CSV>   subtract these from the discovered set
  --lib-dir <DIR>   discovery root (default: ./hoon/lib/)
  --list            print discovered grafts and exit
  --json            machine-readable output (pairs with --list)
  --dry-run         print would-be output to stdout; don't write
```

`--grafts <name>` with a name not present in the discovered set is a hard
error.

### `--list --json` schema

Stable across the v3 plan's lifespan. Tier 2 crates use this at boot to
fail loudly when a required graft is missing.

```json
[
  {
    "name": "settle-graft",
    "version": "0.1.0",
    "priority": 10,
    "blocks": ["imports", "state", "cause", "poke", "peek"],
    "applicable": 5,
    "deferred": false
  }
]
```

Version bumps to this schema append fields, never reshape existing ones.

## Error modes

| Condition | Behavior |
|---|---|
| TOML parse failure | hard error; surface the line number from the parser |
| `[graft]` missing required field (`name`/`version`/`priority`) | hard error |
| `after` references an absent graft | hard error at composition time |
| `--grafts` names an absent graft | hard error |
| Two manifests claim the same `name` | hard error at discovery |
| Marker missing from target file | warning; that marker is skipped, others continue |
| All five markers missing | hard error (nothing to wire) |
| Sentinel found in marker window | skip that graft for that marker; log `skipped` |
| Body contains tabs (mixed indentation) | warning; injection proceeds — `hoonc` may fail downstream |

## Reserved: `[graft.gates]` extension

EXPANSION's gate catalog will introduce a per-graft gate-binding table.
**Reserved; not implemented by graft-inject until the gate catalog
ships.** Documented here so gate-graft authors can write to a stable
target.

```toml
[graft.gates]
gate       = "rag-verifier"          # single named gate from the catalog
gate-chain = ["bind-note-id", "rag"] # composition of named gates, applied left-to-right
```

When implemented, the composer will splice the named gate's body into
the `settle-graft` poke body at inject time, replacing the default
hash-gate. `gate` and `gate-chain` are mutually exclusive — set one or
neither.

## Migration: vesl-graft → settle-graft

Phase 12A (landed) renamed the `vesl-graft` package to `settle-graft`
to align with the four-primitive taxonomy (mint / guard / settle /
forge). The manifest moved from `vesl-graft.toml` to `settle-graft.toml`;
`name`, `sentinel`s, and `body`s updated accordingly. Rust-side helper
functions kept `build_vesl_*_poke` aliases marked `#[deprecated]` for
one release cycle — callers should migrate to `build_settle_*_poke`.
