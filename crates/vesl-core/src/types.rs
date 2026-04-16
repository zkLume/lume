//! Re-exported types for vesl-core consumers.
//!
//! Callers can `use vesl_core::{Ink, Grip, Tip5Hash, ProofNode}`
//! without direct deps on the lower crates.

// tip5 primitives
pub use nockchain_tip5_rs::{
    format_tip5, hash_leaf, hash_pair, tip5_to_atom_le_bytes, verify_proof, MerkleTree, ProofNode,
    Tip5Hash, TIP5_ZERO,
};

// Chain/wallet clients (for Settle/Forge users)
pub use nockchain_client_rs::{ChainClient, ChainConfig, WalletClient, WalletConfig};

// Noun building (for IntentVerifier trait)
pub use nock_noun_rs::NounSlab;

// Vesl domain types — mirrors of sur/vesl.hoon
use serde::{Deserialize, Serialize};

/// Mirror of `+$chunk  [id=chunk-id dat=@t]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: u64,
    pub dat: String,
}

/// Mirror of `+$retrieval  [=chunk proof=merkle-proof score=@ud]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retrieval {
    pub chunk: Chunk,
    pub proof: Vec<ProofNode>,
    pub score: u64,
}

/// Mirror of `+$manifest  [query=@t results=(list retrieval) prompt=@t output=@t page=@ud]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub query: String,
    pub results: Vec<Retrieval>,
    pub prompt: String,
    pub output: String,
    pub page: u64,
}

/// Mirror of `+$nock-zkp  [root=merkle-root prf=@ stamp=@da]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NockZkp {
    pub root: Tip5Hash,
    pub prf: Vec<u8>,
    pub stamp: u64,
}

/// Mirror of `+$note-state`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NoteState {
    Pending,
    Verified(NockZkp),
    Settled,
}

/// Mirror of `+$note  [id=@ hull=hull-id root=merkle-root state=note-state]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub hull: u64,
    pub root: Tip5Hash,
    pub state: NoteState,
}

/// Generic settlement payload — mirrors graft-payload in vesl-graft.hoon.
/// For RAG, `data` is the serialized manifest. For other domains, whatever
/// the verification gate expects.
#[derive(Debug, Clone)]
pub struct GraftPayload {
    pub note: Note,
    pub data: Vec<u8>,
    pub expected_root: Tip5Hash,
}

/// A leaf with its Merkle inclusion proof — generic payload unit for Forge.
/// Mirrors Hoon: `[dat=@ proof=(list [hash=@ side=?])]`
#[derive(Debug, Clone)]
pub struct LeafWithProof {
    pub dat: Vec<u8>,
    pub proof: Vec<ProofNode>,
}

/// Generic STARK proof payload — mirrors forge-kernel.hoon's forge-payload.
/// `[note leaves expected-root]` where leaves carry their own Merkle proofs.
#[derive(Debug, Clone)]
pub struct ForgePayload {
    pub note: Note,
    pub leaves: Vec<LeafWithProof>,
    pub expected_root: Tip5Hash,
}

/// Domain verification trait. Implement for your computation type.
/// `RagVerifier` is the built-in implementation for RAG manifests.
pub trait IntentVerifier: Send + Sync {
    fn verify(&self, data: &[u8], expected_root: &Tip5Hash) -> bool;
    fn build_settle_poke(&self, payload: &GraftPayload) -> anyhow::Result<NounSlab>;
}
