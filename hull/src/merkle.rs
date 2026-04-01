//! Merkle tree engine — re-exports from `vesl-mantle` (which wraps `nockchain-tip5-rs`).
//!
//! The Mantle crate re-exports the full tip5 API. This module provides
//! hull-local access so `crate::merkle::verify_proof` etc. keep working.
//!
//! `tip5_to_atom_le_bytes` comes from the direct `nockchain-tip5-rs` dep
//! since the Mantle doesn't re-export noun-encoding helpers.

pub use vesl_mantle::{
    hash_leaf, hash_pair, verify_proof, MerkleTree, format_tip5,
    Tip5Hash, TIP5_ZERO, ProofNode,
};

// Noun-encoding helper — not re-exported by Mantle (implementation detail).
pub use nockchain_tip5_rs::tip5_to_atom_le_bytes;
