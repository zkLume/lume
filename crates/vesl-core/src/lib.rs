//! vesl-core — High-level Vesl SDK
//!
//! Four primitives, each a different weight class:
//!
//! - **Mint** — Data commitment. Pure math, zero async. Commit chunks, get a root.
//! - **Guard** — Verification. Prove chunks and manifests against trusted roots.
//! - **Settle** — Settlement. Kernel boot + chain access for note state transitions.
//! - **Forge** — STARK proof. Everything Settle does, plus proof generation.
//!
//! Callers pick the primitive they need. Mint users never touch the kernel.
//! Forge users get the full pipeline.

pub mod settle;
pub mod config;
pub mod noun_builder;
pub mod tx_builder;
pub mod guard;
pub mod mint;
pub mod forge;
pub mod signing;
pub mod types;

// Top-level re-exports so callers can write:
//   use vesl_core::{Mint, Guard, Tip5Hash, ProofNode};
pub use mint::Mint;
pub use guard::Guard;
pub use settle::Settle;
pub use forge::Forge;

pub use types::{
    Chunk, Manifest, Note, NockZkp, NoteState, Retrieval,
    Tip5Hash, ProofNode, TIP5_ZERO, MerkleTree,
    ChainClient, ChainConfig, WalletClient, WalletConfig,
    format_tip5, hash_leaf, hash_pair, tip5_to_atom_le_bytes, verify_proof,
    GraftPayload, IntentVerifier, NounSlab,
    ForgePayload, LeafWithProof,
};
pub use guard::GuardError;
pub use mint::MintError;
pub use settle::RagVerifier;
pub use signing::{SigningError, derive_pubkey, pubkey_hash, sign, key_from_seed_phrase};
pub use config::{SettlementMode, SettlementConfig, SettlementToml};
