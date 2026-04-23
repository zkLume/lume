//! Settle-graft poke builders.
//!
//! Tagged in the graft namespace (`%settle-register`, `%settle-note`,
//! `%settle-verify`), distinct from the RAG-flavored `%register` /
//! `%settle` / `%prove` pokes in `crate::settle`. Use these when wiring
//! a kernel that was composed via `graft-inject` — the marker injection
//! emits `?-` arms matching these tags.
//!
//! Hull IDs are routed through `atom_from_u64` so callers can pass
//! hash-derived hull IDs above `DIRECT_MAX` without crashing the noun
//! constructor. Note IDs likewise — settled note IDs are usually
//! `hash-leaf(jam(payload))` which exceeds `DIRECT_MAX`.
//!
//! The wire shape (`[%settle-* payload=@]`) is unchanged by the H-03
//! audit fix to `verify-gate` — the graft itself extracts `note-id`
//! from the payload and passes it to the gate.
//!
//! Phase 12A renamed the primitive from `vesl-graft` to `settle-graft`
//! across the Hoon + Rust surface. Deprecated `build_vesl_*_poke`
//! aliases are kept for one release cycle in `crate::lib`.
//!  `%vesl-settle` → `%settle-note` drops the tautological
//! `%settle-settle` and avoids collision with `%mint-commit`.

use nock_noun_rs::{
    atom_from_u64, jam_to_bytes, make_atom_in, make_tag_in, new_stack, NounSlab,
};
use nockchain_tip5_rs::{tip5_to_atom_le_bytes, Tip5Hash};
use nockvm::noun::{Noun, T};

/// Build a `[%settle-register hull=@ root=@]` poke.
///
/// Pair with the `%settle-register` arm installed by `graft-inject`.
pub fn build_settle_register_poke(hull: u64, root: &Tip5Hash) -> NounSlab {
    let mut slab = NounSlab::new();
    let tag = make_tag_in(&mut slab, "settle-register");
    let hull_noun = atom_from_u64(&mut slab, hull);
    let root_bytes = tip5_to_atom_le_bytes(root);
    let root_noun = make_atom_in(&mut slab, &root_bytes);
    let poke = T(&mut slab, &[tag, hull_noun, root_noun]);
    slab.set_root(poke);
    slab
}

/// Build a `[%settle-note jammed-graft-payload]` poke for a single-leaf
/// commitment.
///
/// `data` is the raw payload bytes the default hash-gate will hash and
/// compare against the registered root. For single-leaf commits, the
/// registered root equals `hash-leaf(data)`.
pub fn build_settle_note_poke(
    note_id: u64,
    hull: u64,
    root: &Tip5Hash,
    data: &[u8],
) -> NounSlab {
    build_settle_payload_poke("settle-note", note_id, hull, root, data)
}

/// Build a `[%settle-verify jammed-graft-payload]` poke for a single-leaf
/// commitment. Same payload shape as `settle-note` but pure verification:
/// no state transition, no replay check.
pub fn build_settle_verify_poke(
    note_id: u64,
    hull: u64,
    root: &Tip5Hash,
    data: &[u8],
) -> NounSlab {
    build_settle_payload_poke("settle-verify", note_id, hull, root, data)
}

// -- deprecated aliases (Phase 12A) --------------------------------------

/// Deprecated alias for [`build_settle_register_poke`].
#[deprecated(
    since = "0.6.0",
    note = "renamed in Phase 12A; use build_settle_register_poke"
)]
pub fn build_vesl_register_poke(hull: u64, root: &Tip5Hash) -> NounSlab {
    build_settle_register_poke(hull, root)
}

/// Deprecated alias for [`build_settle_note_poke`] (the `%vesl-settle`
/// cause became `%settle-note` in Phase 12A — the new name emphasizes
/// "settle the note" and avoids the tautological `%settle-settle`).
#[deprecated(
    since = "0.6.0",
    note = "renamed in Phase 12A; use build_settle_note_poke"
)]
pub fn build_vesl_settle_poke(
    note_id: u64,
    hull: u64,
    root: &Tip5Hash,
    data: &[u8],
) -> NounSlab {
    build_settle_note_poke(note_id, hull, root, data)
}

/// Deprecated alias for [`build_settle_verify_poke`].
#[deprecated(
    since = "0.6.0",
    note = "renamed in Phase 12A; use build_settle_verify_poke"
)]
pub fn build_vesl_verify_poke(
    note_id: u64,
    hull: u64,
    root: &Tip5Hash,
    data: &[u8],
) -> NounSlab {
    build_settle_verify_poke(note_id, hull, root, data)
}

fn build_settle_payload_poke(
    verb: &str,
    note_id: u64,
    hull: u64,
    root: &Tip5Hash,
    data: &[u8],
) -> NounSlab {
    let mut slab = NounSlab::new();
    let tag = make_tag_in(&mut slab, verb);
    let payload = build_graft_single_leaf_payload_in(&mut slab, note_id, hull, root, data);
    let payload_bytes = {
        let mut stack = new_stack();
        jam_to_bytes(&mut stack, payload)
    };
    let jammed = make_atom_in(&mut slab, &payload_bytes);
    let poke = T(&mut slab, &[tag, jammed]);
    slab.set_root(poke);
    slab
}

/// Build a single-leaf `graft-payload` noun in `slab`. Shape matches
/// `settle-graft.hoon`'s `+$graft-payload`:
///
/// ```text
/// [note=[id=@ hull=@ root=@ state=[%pending ~]] data=@ expected-root=@]
/// ```
fn build_graft_single_leaf_payload_in(
    slab: &mut NounSlab,
    note_id: u64,
    hull: u64,
    root: &Tip5Hash,
    data: &[u8],
) -> Noun {
    let root_bytes = tip5_to_atom_le_bytes(root);
    let note_root = make_atom_in(slab, &root_bytes);
    let pending = make_tag_in(slab, "pending");
    let state = T(slab, &[pending, nockvm::noun::D(0)]);
    let id = atom_from_u64(slab, note_id);
    let hull_atom = atom_from_u64(slab, hull);
    let note = T(slab, &[id, hull_atom, note_root, state]);
    let data_atom = make_atom_in(slab, data);
    let exp_root = make_atom_in(slab, &root_bytes);
    T(slab, &[note, data_atom, exp_root])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mint;
    use nock_noun_rs::slab_root;

    fn fixture_root() -> Tip5Hash {
        let data: [&[u8]; 1] = [b"hello world"];
        let mut mint = Mint::new();
        mint.commit(&data)
    }

    #[test]
    fn build_settle_register_poke_emits_nonempty_jam() {
        let slab = build_settle_register_poke(1, &fixture_root());
        let mut stack = new_stack();
        let bytes = jam_to_bytes(&mut stack, slab_root(&slab));
        assert!(!bytes.is_empty());
    }

    #[test]
    fn build_settle_note_poke_emits_nonempty_jam() {
        let root = fixture_root();
        let slab = build_settle_note_poke(101, 1, &root, b"hello world");
        let mut stack = new_stack();
        let bytes = jam_to_bytes(&mut stack, slab_root(&slab));
        assert!(!bytes.is_empty());
    }

    #[test]
    fn build_settle_verify_poke_emits_nonempty_jam() {
        let root = fixture_root();
        let slab = build_settle_verify_poke(101, 1, &root, b"hello world");
        let mut stack = new_stack();
        let bytes = jam_to_bytes(&mut stack, slab_root(&slab));
        assert!(!bytes.is_empty());
    }

    #[test]
    fn note_and_verify_pokes_share_payload_bytes() {
        // Same wire shape, only the verb tag differs — the jammed payload
        // bytes inside the cell must match.
        let root = fixture_root();
        let s = build_settle_note_poke(7, 3, &root, b"x");
        let v = build_settle_verify_poke(7, 3, &root, b"x");

        // Pull the second slot (the jammed payload) from each NounSlab.
        let s_payload = unsafe {
            let cell = (*s.root()).as_cell().expect("note poke is a cell");
            jam_to_bytes(&mut new_stack(), cell.tail())
        };
        let v_payload = unsafe {
            let cell = (*v.root()).as_cell().expect("verify poke is a cell");
            jam_to_bytes(&mut new_stack(), cell.tail())
        };
        assert_eq!(s_payload, v_payload);
    }

    #[test]
    fn large_hull_id_does_not_panic() {
        // hash-derived hulls routinely exceed DIRECT_MAX (2^63 - 1).
        // atom_from_u64 must route through a real atom alloc.
        let hull = u64::MAX - 7;
        let _slab = build_settle_register_poke(hull, &fixture_root());
        let _slab2 = build_settle_note_poke(u64::MAX - 11, hull, &fixture_root(), b"x");
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_aliases_match_canonical_output() {
        let root = fixture_root();
        let a = build_vesl_register_poke(1, &root);
        let b = build_settle_register_poke(1, &root);
        let ab = jam_to_bytes(&mut new_stack(), slab_root(&a));
        let bb = jam_to_bytes(&mut new_stack(), slab_root(&b));
        assert_eq!(ab, bb);

        let c = build_vesl_settle_poke(7, 3, &root, b"x");
        let d = build_settle_note_poke(7, 3, &root, b"x");
        let cb = jam_to_bytes(&mut new_stack(), slab_root(&c));
        let db = jam_to_bytes(&mut new_stack(), slab_root(&d));
        assert_eq!(cb, db);
    }
}
