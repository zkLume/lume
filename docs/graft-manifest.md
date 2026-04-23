# `graft.toml` schema

A graft manifest describes how `graft-inject` composes a Hoon library
into a host kernel's `app.hoon`. One manifest per graft, sibling to the
graft's `.hoon` file under `protocol/lib/` (or `hoon/lib/` after `sync.sh`
in vesl-nockup).

This document is the source of truth for the manifest format. The Rust
loader in `vesl-nockup/tools/graft-inject` implements it; graft authors
read this to write a manifest without reading the loader.

## Trust model

A manifest's `body` field is Hoon text pasted **verbatim** into the
developer's `app.hoon`. `graft-inject` does not sanitize, sandbox, check
signatures on, or verify the provenance of the manifest. Whatever Hoon
a `.toml` declares becomes kernel source on the next invocation.

Consequences:

- Manifests are code. Treat them like any other dependency: review
  incoming changes the way you would a PR that touches `protocol/lib/`.
- `graft-inject` is a composition step, not a trust boundary. Trust is
  managed at the distribution layer — checkout provenance, directory
  hygiene, what lands in `hoon/lib/` via `sync.sh` or manual edits.
- As the AUDIT 2026-04-19 H-10 write-up spells out, `graft-inject`
  defaults to **preview-only** — the composed diff and a sha256 per
  manifest print to stderr, and `--apply` is required to write. This
  keeps silent supply-chain drift impossible without explicit consent.

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
| `sentinel` | string | yes | Documentation-only after AUDIT 2026-04-19. The loader used to scan for this substring to detect already-injected wiring; idempotence now runs off `::  graft-inject:<name>:<marker>:begin` banner comments the composer emits. `sentinel` still names the field's canonical marker — useful for authors and reviewers reading the manifest — but carries no behavior. |
| `body` | string | yes | The Hoon to paste at the marker. Stored unindented; the loader re-applies indentation from the marker line's leading whitespace. Leading and trailing newlines on the string are trimmed before injection. |

### Sentinel rules

The sentinel is documentation; the loader does not read it for
idempotence. Authors should still pick a short, unambiguous marker so
reviewers reading the manifest can map a graft to its canonical
injected line.

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

Each injected block — regardless of marker — is wrapped in a per-graft
begin/end banner pair:

```hoon
::  graft-inject:<graft-name>:<marker>:begin
<composed body lines>
::  graft-inject:<graft-name>:<marker>:end
```

The banners are the idempotence signal (see below). They also read as
useful provenance when a reviewer is scanning a composed `app.hoon` —
every injected block is attributable to its manifest at a glance.

### Non-peek markers

Each graft's `body` is wrapped in banners and concatenated in priority
order.

### `peek` marker

A peek chain. Each graft contributes a banner-wrapped pair:

```hoon
::  graft-inject:<name>:peek:begin
=/  <stub>-res  <peek.body>
?.  =(~ <stub>-res)  <stub>-res
::  graft-inject:<name>:peek:end
```

The terminal `~` from the bare scaffold remains as the chain's final
fallback. A graft's peek body must return `~` (not `[~ ~]`) for paths
it doesn't handle, so the chain falls through to the next graft.

### Idempotence

- **Per-graft-per-marker**: re-running `graft-inject` scans the file
  for exact trimmed-line matches against `::  graft-inject:<name>:<marker>:begin`.
  If found, the graft is considered already wired at that marker and
  skipped; other grafts and other markers are evaluated independently.
- **Peek-chain**: new grafts' banner-wrapped pairs land immediately
  before the last bare `~` between the peek marker and its block's
  closing `==`. The window is unbounded within the block (AUDIT
  2026-04-19 H-13 fix), so chains grow safely past any size.
- **No overwrite**: removing a graft from `--grafts` does NOT remove
  its existing banner block. The tool is additive by design; cleanup
  is a manual operation.

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
  --apply           write the composed output to PATH (default: preview-only)
  --dry-run         deprecated alias of the default preview-only behavior
```

Default behavior (no `--apply`): `graft-inject` prints the composed
output to stdout and a per-manifest sha256 summary + "add --apply to
write" hint to stderr. `--apply` is required to write to disk. See the
Trust model section above for the reasoning.

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
    "deferred": false,
    "sha256": "a9c72bbe…"
  }
]
```

`sha256` is the hex sha256 of the manifest's raw TOML bytes — added per
AUDIT 2026-04-19 H-10 so supply-chain reviewers can pin expected digests
without re-reading the file.

Version bumps to this schema append fields, never reshape existing ones.

## Error modes

| Condition | Behavior |
|---|---|
| TOML parse failure | hard error; surface the line number from the parser |
| `[graft]` missing required field (`name`/`version`/`priority`) | hard error |
| `name` not matching `^[a-z][a-z0-9-]*$` | hard error at discovery |
| `after` references an absent graft | hard error at discovery |
| `--grafts` names an absent graft | hard error |
| Two manifests claim the same `name` | hard error at discovery; both source paths named in the message |
| Marker missing from target file | warning; that marker is skipped, others continue |
| All five markers missing | hard error (nothing to wire) |
| Banner `::  graft-inject:<name>:<marker>:begin` already present | skip that graft-marker pair; log `skipped` |
| Body contains tabs (mixed indentation) | injection proceeds — `hoonc` may fail downstream |

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
