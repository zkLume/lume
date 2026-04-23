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

// Noun building (for CommitmentVerifier trait)
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

/// Generic settlement payload — mirrors graft-payload in settle-graft.hoon.
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

/// Commitment verification trait. Implement for your computation type.
/// `RagVerifier` is the built-in implementation for RAG manifests.
///
/// Decides whether `data` binds to `expected_root` under a domain-specific
/// rule (for RAG: manifest chunks prove into the merkle root; for other
/// domains: whatever the commitment gate demands). This is commitment-layer
/// plumbing — it has nothing to do with intent coordination despite the
/// legacy `IntentVerifier` name retained as a deprecated alias below.
///
/// AUDIT 2026-04-17 H-03: `verify` takes `note_id` so domain verifiers
/// can enforce `note_id == deterministic_fn(data)`, closing the
/// pre-commit race where an attacker predicts a victim's note-id and
/// settles a different manifest under it first. Implementations that
/// don't care about note-id binding can simply ignore the argument.
pub trait CommitmentVerifier: Send + Sync {
    fn verify(&self, note_id: u64, data: &[u8], expected_root: &Tip5Hash) -> bool;
    fn build_settle_poke(&self, payload: &GraftPayload) -> anyhow::Result<NounSlab>;
}

/// Deprecated alias for `CommitmentVerifier`. The original name conflated
/// intent coordination with commitment verification — see
/// `.dev/BIFURCATE_INTENT.md` and `.dev/GRAFT_REFACTOR.md` for the taxonomy
/// cleanup. Will be removed in the next minor release.
#[deprecated(note = "renamed to CommitmentVerifier; IntentVerifier will be removed in the next minor release")]
pub use self::CommitmentVerifier as IntentVerifier;
