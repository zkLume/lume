//! Merkle tree engine — re-exports from `nockchain-tip5-rs` crate.
//!
//! The standalone crate contains the full implementation:
//! tip5 hash functions, Merkle tree construction, proof generation,
//! and proof verification. Mathematical mirror of `vesl-logic.hoon`.

pub use nockchain_tip5_rs::{
    hash_leaf, hash_pair, verify_proof, MerkleTree, format_tip5,
    tip5_to_atom_le_bytes, Tip5Hash, TIP5_ZERO, ProofNode,
};
