//! vesl-mantle — High-level Vesl SDK
//!
//! Four tiers, each a different weight class:
//!
//! - **Sigil** — Data commitment. Pure math, zero async. Commit chunks, get a root.
//! - **Vigil** — Verification. Prove chunks and manifests against trusted roots.
//! - **Anchor** — Settlement. Kernel boot + chain access for note state transitions.
//! - **Kraken** — STARK proof. Everything Anchor does, plus proof generation.
//!
//! Callers pick the tier they need. Sigil users never touch the kernel.
//! Kraken users get the full pipeline.

pub mod anchor;
pub mod vigil;
pub mod sigil;
pub mod kraken;
pub mod types;

// Top-level re-exports so callers can write:
//   use vesl_mantle::{Sigil, Vigil, Tip5Hash, ProofNode};
pub use sigil::Sigil;
pub use vigil::Vigil;
pub use anchor::Anchor;
pub use kraken::Kraken;

pub use types::{
    Chunk, Manifest, Note, NockZkp, NoteState, Retrieval,
    Tip5Hash, ProofNode, TIP5_ZERO, MerkleTree,
    ChainClient, ChainConfig, WalletClient, WalletConfig,
    format_tip5, hash_leaf, hash_pair, verify_proof,
    GraftPayload, IntentVerifier, NounSlab,
};
pub use anchor::RagVerifier;
