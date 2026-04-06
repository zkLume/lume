# nock-noun-rs

> You shouldn't need to read 57,000 lines of wallet code to build a cell.

Ergonomic Nock noun construction from Rust. The missing manual for
`NockStack`, `NounSlab`, and the sharp edges between them.

## Why this exists

Every NockApp developer hits the same wall: you need to build Nock nouns
from Rust, but the only reference implementation lives inside a monorepo
the size of a small country. The APIs are powerful, undocumented, and will
silently corrupt your heap if you look at them wrong.

This crate is the aspirin.

## Quick start

```rust
use nock_noun_rs::*;

let mut stack = new_stack();

// Build a tagged cell: [%hello 'world']
let tag  = make_tag(&mut stack, "hello");
let body = make_cord(&mut stack, "world");
let cell = T(&mut stack, &[tag, body]);

// Jam to bytes for wire transmission
let bytes = jam_to_bytes(&mut stack, cell);
```

## What you get

| Function | What it does | Hoon equivalent |
|----------|-------------|-----------------|
| `new_stack()` | 8 MB NockStack arena | `=/ stack (new ...)` |
| `make_atom(stack, &[u8])` | Byte slice -> atom (LE) | Bare `@` |
| `make_cord(stack, "text")` | UTF-8 string -> `@t` | `'text'` |
| `make_tag(stack, "name")` | ASCII string -> `@tas` | `%name` |
| `make_loobean(bool)` | `true` -> `D(0)`, `false` -> `D(1)` | `%.y` / `%.n` |
| `make_list(stack, &[Noun])` | Null-terminated list | `~[a b c]` |
| `jam_to_bytes(stack, noun)` | Serialize noun | `(jam noun)` |
| `cue_from_bytes(stack, &[u8])` | Deserialize noun | `(cue atom)` |

Every function also has a `_in` variant (e.g. `make_atom_in`) that works
with any `NounAllocator` — use these with `NounSlab` for building poke
causes.

## The traps nobody warns you about

**Loobeans are inverted.** Hoon `%.y` (yes/true) is atom `0`. Hoon `%.n`
(no/false) is atom `1`. This is not a mistake. This is a feature. If you
write `if flag { D(1) } else { D(0) }` you will spend three hours debugging
a silent logic inversion. Use `make_loobean()` and move on with your life.

**Cords are not strings.** A Hoon cord (`@t`) is an atom whose bytes
happen to be valid UTF-8. `"abc"` becomes `97 + 98*256 + 99*65536 =
6513249`. The bytes are the atom. The atom is the bytes. There is no
encoding step. There is no decoding step. This is beautiful once you
see it and confusing until you do.

**Lists are null-terminated.** `~[1 2 3]` in Hoon is `[1 [2 [3 0]]]` in
Nock. Not `[1 2 3]`. Not `[1 [2 [3]]]`. The zero matters. `make_list()`
handles this. If you build lists by hand, you will forget the zero
exactly once.

**NounSlab and NockStack are not interchangeable.** NockStack is an arena
for building a noun tree you'll jam and throw away. NounSlab is an arena
for building a noun you'll hand to the NockApp framework. Mixing them up
doesn't crash. It corrupts memory in ways that surface three function
calls later. The `_in` variants exist so you don't have to think about
this.

## Re-exports

This crate re-exports the essentials so you don't need to depend on
`nockvm` and `nockapp` directly:

```rust
pub use nockvm::mem::NockStack;
pub use nockvm::noun::{Cell, D, IndirectAtom, Noun, NounAllocator, T};
pub use nockvm::serialization::{cue, jam};
pub use nockapp::noun::slab::NounSlab;
```

## Who made this

Extracted from [Vesl](https://github.com/zkVesl/vesl) — verifiable RAG
on Nockchain. Built by people who misuse `NounSlab` in production and
have the scars to prove it.

Part of the Nockchain ecosystem. `~`
