//! Rust mirrors of the Hoon data structures from protocol/sur/vesl.hoon.
//!
//! Every type here maps 1:1 to a Nock noun defined in the protocol layer.
//! Atoms (@) become fixed-width Rust integers or byte arrays.
//! Cords (@t) become String (UTF-8, matching Hoon cord byte layout).
//! Looleans (?) become bool (%.y=true, %.n=false).
//!
//! Hash fields use `[u64; 5]` — native tip5 noun-digest representation.
//! Each limb is a Goldilocks field element (u64 < 2^64 - 2^32 + 1).
//! Converted to flat `@` atoms for noun encoding via base-p polynomial.

use serde::{Deserialize, Serialize};

// Re-export tip5 types from the standalone crate.
pub use nockchain_tip5_rs::{Tip5Hash, TIP5_ZERO, ProofNode};

// ---------------------------------------------------------------------------
// Tier 1: Sovereign Storage
// ---------------------------------------------------------------------------

/// Mirror of `+$chunk  [id=chunk-id dat=@t]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: u64,
    pub dat: String,
}

// ---------------------------------------------------------------------------
// Tier 2: Local Inference
// ---------------------------------------------------------------------------

/// Mirror of `+$retrieval  [=chunk proof=merkle-proof score=@ud]`
///
/// score: fixed-point integer (multiply by 10^6). ZK arithmetic is integer-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retrieval {
    pub chunk: Chunk,
    pub proof: Vec<ProofNode>,
    pub score: u64,
}

/// Mirror of `+$manifest  [query=@t results=(list retrieval) prompt=@t output=@t]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub query: String,
    pub results: Vec<Retrieval>,
    pub prompt: String,
    pub output: String,
}

// ---------------------------------------------------------------------------
// Tier 3: Nock-Prover
// ---------------------------------------------------------------------------

/// Mirror of `+$nock-zkp  [root=merkle-root prf=@ stamp=@da]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NockZkp {
    pub root: Tip5Hash,
    pub prf: Vec<u8>,
    pub stamp: u64,
}

// ---------------------------------------------------------------------------
// Tier 4: Settlement
// ---------------------------------------------------------------------------

/// Mirror of `+$note-state  $%  [%pending ~] [%verified p=nock-zkp] [%settled ~]  ==`
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
