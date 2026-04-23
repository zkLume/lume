//! Mint-graft poke builders.
//!
//! The mint graft is the lightest commitment primitive: store a
//! Merkle root under a hull-id, no gate, no verify, no settlement.
//! Pair with the `%mint-commit` arm installed by `graft-inject`.
//!
//! Hull IDs route through `atom_from_u64` so callers can pass
//! hash-derived hulls above `DIRECT_MAX` without crashing the noun
//! constructor.

use nock_noun_rs::{atom_from_u64, make_atom_in, make_tag_in, NounSlab};
use nockchain_tip5_rs::{tip5_to_atom_le_bytes, Tip5Hash};
use nockvm::noun::T;

/// Build a `[%mint-commit hull=@ root=@]` poke.
pub fn build_mint_commit_poke(hull: u64, root: &Tip5Hash) -> NounSlab {
    let mut slab = NounSlab::new();
    let tag = make_tag_in(&mut slab, "mint-commit");
    let hull_noun = atom_from_u64(&mut slab, hull);
    let root_bytes = tip5_to_atom_le_bytes(root);
    let root_noun = make_atom_in(&mut slab, &root_bytes);
    let poke = T(&mut slab, &[tag, hull_noun, root_noun]);
    slab.set_root(poke);
    slab
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mint;
    use nock_noun_rs::{jam_to_bytes, new_stack, slab_root};

    fn fixture_root() -> Tip5Hash {
        let data: [&[u8]; 1] = [b"hello world"];
        let mut mint = Mint::new();
        mint.commit(&data)
    }

    #[test]
    fn build_mint_commit_poke_emits_nonempty_jam() {
        let slab = build_mint_commit_poke(1, &fixture_root());
        let mut stack = new_stack();
        let bytes = jam_to_bytes(&mut stack, slab_root(&slab));
        assert!(!bytes.is_empty());
    }

    #[test]
    fn large_hull_id_does_not_panic() {
        // hash-derived hulls routinely exceed DIRECT_MAX (2^63 - 1).
        let hull = u64::MAX - 7;
        let _slab = build_mint_commit_poke(hull, &fixture_root());
    }
}
