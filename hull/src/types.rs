//! Rust mirrors of the Hoon data structures from protocol/sur/vesl.hoon.
//!
//! Canonical definitions live in `vesl-mantle`. This module re-exports them
//! so existing `use crate::types::*` imports throughout hull continue to work.

pub use vesl_mantle::{
    Chunk, Manifest, Note, NockZkp, NoteState, Retrieval,
    Tip5Hash, TIP5_ZERO, ProofNode,
};
