//! vesl-mantle — High-level Vesl SDK
//!
//! Four tentacles, each a different weight class:
//!
//! - **Ink** — Data commitment. Pure math, zero async. Commit chunks, get a root.
//! - **Grip** — Verification. Prove chunks and manifests against trusted roots.
//! - **Beak** — Settlement. Kernel boot + chain access for note state transitions.
//! - **Kraken** — STARK proof. Everything Beak does, plus proof generation.
//!
//! Callers pick the tentacle they need. Ink users never touch the kernel.
//! Kraken users get the full pipeline.

pub mod beak;
pub mod grip;
pub mod ink;
pub mod kraken;
pub mod types;

// Top-level re-exports so callers can write:
//   use vesl_mantle::{Ink, Grip, Tip5Hash, ProofNode};
pub use ink::Ink;
pub use grip::Grip;
pub use beak::Beak;
pub use kraken::Kraken;

pub use types::{
    Chunk, Manifest, Note, NockZkp, NoteState, Retrieval,
    Tip5Hash, ProofNode, TIP5_ZERO, MerkleTree,
    ChainClient, ChainConfig, WalletClient, WalletConfig,
    format_tip5, hash_leaf, hash_pair, verify_proof,
};
