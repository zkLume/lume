//! NoteData encoding and decoding helpers.
//!
//! Nockchain's NoteV1 carries `NoteData` — a list of key-value entries where
//! values are JAM-encoded Nock nouns. Every NockApp that puts structured data
//! on-chain needs to encode to and decode from this format.
//!
//! # Encoding
//!
//! ```ignore
//! use nockchain_client_rs::note_data::{jam_u64_entry, jam_tip5_entry};
//!
//! let version_entry = jam_u64_entry("my-app-v", 1);
//! let hash_entry = jam_tip5_entry("my-app-root", &merkle_root);
//! let note_data = NoteData::new(vec![version_entry, hash_entry]);
//! ```
//!
//! # Decoding
//!
//! ```ignore
//! use nockchain_client_rs::note_data::{find_u64_entry, find_hash_entry};
//!
//! let version = find_u64_entry(&note_data, "my-app-v")?;
//! let root = find_hash_entry(&note_data, "my-app-root")?;
//! ```
//!
//! # Tip5 Hash Encoding
//!
//! Tip5 hashes (`[u64; 5]`) are encoded as null-terminated Nock lists:
//! `[limb0 limb1 limb2 limb3 limb4 0]`. Each limb is a Belt-sized u64 value.

use anyhow::{Context, Result};
use nockapp::noun::slab::{NockJammer, NounSlab};
use nockchain_tip5_rs::Tip5Hash;
use nockchain_types::tx_engine::v1::note::{NoteData, NoteDataEntry};
use nockvm::noun::{IndirectAtom, Noun, D, T};

/// Dereference a NounSlab's root noun (C-001 / AUDIT H-06).
///
/// Centralized local helper so the `unsafe` block is in one place per
/// crate. The caller must ensure `slab.set_root(..)` was called (or the
/// slab was populated via `cue_into` / `NockApp::poke`, both of which
/// set the root internally). The returned `Noun` may contain raw
/// pointers into the slab's arena and must not outlive it.
///
/// This crate does not depend on `nock-noun-rs` to avoid a cycle with
/// the rest of the vesl stack; the identical helper in nock-noun-rs
/// is the canonical one for nock-noun-rs consumers.
fn slab_root(slab: &NounSlab) -> Noun {
    // SAFETY: copied out by value; never stored as `&Noun` past this
    // function call. See the doc comment for the set_root contract.
    unsafe { *slab.root() }
}

// ---------------------------------------------------------------------------
// Encoding — Rust values to jammed NoteDataEntry
// ---------------------------------------------------------------------------

/// Create a NoteDataEntry with a jammed u64 atom value.
pub fn jam_u64_entry(key: &str, value: u64) -> NoteDataEntry {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    // D() panics on values > DIRECT_MAX; u64_to_noun (below) handles both.
    let noun = u64_to_noun(&mut slab, value);
    slab.set_root(noun);
    let jammed = slab.jam();
    NoteDataEntry::new(key.to_string(), jammed)
}

/// Create a NoteDataEntry with a jammed tip5 hash value.
///
/// Encodes the `[u64; 5]` digest as a null-terminated list of 5 u64 atoms:
/// `[limb0 limb1 limb2 limb3 limb4 0]`.
pub fn jam_tip5_entry(key: &str, hash: &Tip5Hash) -> NoteDataEntry {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let mut noun = D(0); // null terminator
    for &limb in hash.iter().rev() {
        let limb_noun = u64_to_noun(&mut slab, limb);
        noun = T(&mut slab, &[limb_noun, noun]);
    }
    slab.set_root(noun);
    let jammed = slab.jam();
    NoteDataEntry::new(key.to_string(), jammed)
}

/// Create a NoteDataEntry with a jammed opaque byte blob.
///
/// The input `raw_bytes` (typically already-JAM'd proof bytes) are wrapped as a
/// single `IndirectAtom` and then JAM'd. When the chain CUEs this blob it sees
/// one atom — 1 leaf in the z-map tree — regardless of the byte length.
pub fn jam_opaque_bytes_entry(key: &str, raw_bytes: &[u8]) -> NoteDataEntry {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let noun = if raw_bytes.is_empty() {
        D(0)
    } else {
        unsafe {
            let mut indirect = IndirectAtom::new_raw_bytes_ref(&mut slab, raw_bytes);
            indirect.normalize_as_atom().as_noun()
        }
    };
    slab.set_root(noun);
    let jammed = slab.jam();
    NoteDataEntry::new(key.to_string(), jammed)
}

/// Convert a u64 to a Nock noun, using IndirectAtom for values > DIRECT_MAX.
///
/// Nock's `D()` constructor only handles values up to 2^63 - 1. Values above
/// that threshold require indirect atom allocation.
pub fn u64_to_noun(slab: &mut NounSlab<NockJammer>, val: u64) -> Noun {
    const DIRECT_MAX: u64 = (1u64 << 63) - 1;
    if val <= DIRECT_MAX {
        D(val)
    } else {
        let bytes = val.to_le_bytes();
        unsafe {
            let mut indirect = IndirectAtom::new_raw_bytes_ref(slab, &bytes);
            indirect.normalize_as_atom().as_noun()
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding — jammed NoteDataEntry to Rust values
// ---------------------------------------------------------------------------

/// Find a NoteDataEntry by key and decode its jammed value as a u64.
pub fn find_u64_entry(data: &NoteData, key: &str) -> Result<u64> {
    let entry = find_entry(data, key)?;
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    slab.cue_into(entry.blob.clone())
        .context("failed to cue NoteDataEntry blob")?;
    let noun = slab_root(&slab);
    let atom = noun
        .as_atom()
        .map_err(|_| anyhow::anyhow!("expected atom for key '{key}', got cell"))?;
    atom.as_u64()
        .map_err(|_| anyhow::anyhow!("atom for key '{key}' does not fit in u64"))
}

/// Find a NoteDataEntry by key and decode its jammed value as a tip5 hash.
///
/// Reads a 5-element Nock list `[limb0 limb1 limb2 limb3 limb4 0]` and
/// reconstructs the `[u64; 5]` digest.
pub fn find_hash_entry(data: &NoteData, key: &str) -> Result<Tip5Hash> {
    let entry = find_entry(data, key)?;
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    slab.cue_into(entry.blob.clone())
        .context("failed to cue NoteDataEntry blob")?;
    let mut noun = slab_root(&slab);
    let mut limbs = [0u64; 5];
    for (i, limb) in limbs.iter_mut().enumerate() {
        let cell = noun.as_cell().map_err(|_| {
            anyhow::anyhow!("tip5 hash list too short at index {i} for key '{key}'")
        })?;
        let atom = cell.head().as_atom().map_err(|_| {
            anyhow::anyhow!("tip5 limb {i} is not an atom for key '{key}'")
        })?;
        *limb = atom
            .as_u64()
            .map_err(|_| anyhow::anyhow!("tip5 limb {i} exceeds u64 for key '{key}'"))?;
        noun = cell.tail();
    }
    Ok(limbs)
}

/// Find a NoteDataEntry by key and decode its jammed value as raw bytes.
///
/// Inverse of `jam_opaque_bytes_entry`. CUEs the blob, extracts the atom,
/// and returns the original byte content. The zero atom decodes to an empty vec.
pub fn find_opaque_bytes_entry(data: &NoteData, key: &str) -> Result<Vec<u8>> {
    let entry = find_entry(data, key)?;
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    slab.cue_into(entry.blob.clone())
        .context("failed to cue NoteDataEntry blob")?;
    let noun = slab_root(&slab);
    let atom = noun
        .as_atom()
        .map_err(|_| anyhow::anyhow!("expected atom for key '{key}', got cell"))?;
    let bytes = atom.as_ne_bytes();
    let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
    Ok(bytes[..len].to_vec())
}

/// Find a NoteDataEntry by its key string.
pub fn find_entry<'a>(data: &'a NoteData, key: &str) -> Result<&'a NoteDataEntry> {
    data.iter()
        .find(|e| e.key == key)
        .ok_or_else(|| anyhow::anyhow!("NoteData key '{key}' not found"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u64_roundtrip() {
        let entry = jam_u64_entry("test-key", 42);
        let data = NoteData::new(vec![entry]);
        let decoded = find_u64_entry(&data, "test-key").unwrap();
        assert_eq!(decoded, 42);
    }

    #[test]
    fn u64_zero_roundtrip() {
        let entry = jam_u64_entry("zero", 0);
        let data = NoteData::new(vec![entry]);
        assert_eq!(find_u64_entry(&data, "zero").unwrap(), 0);
    }

    #[test]
    fn u64_max_direct_roundtrip() {
        let max_direct = (1u64 << 63) - 1;
        let entry = jam_u64_entry("max", max_direct);
        let data = NoteData::new(vec![entry]);
        assert_eq!(find_u64_entry(&data, "max").unwrap(), max_direct);
    }

    #[test]
    fn tip5_hash_roundtrip() {
        let hash: Tip5Hash = [1, 2, 3, 4, 5];
        let entry = jam_tip5_entry("root", &hash);
        let data = NoteData::new(vec![entry]);
        let decoded = find_hash_entry(&data, "root").unwrap();
        assert_eq!(decoded, hash);
    }

    #[test]
    fn tip5_hash_zero_roundtrip() {
        let hash: Tip5Hash = [0, 0, 0, 0, 0];
        let entry = jam_tip5_entry("zero-root", &hash);
        let data = NoteData::new(vec![entry]);
        assert_eq!(find_hash_entry(&data, "zero-root").unwrap(), hash);
    }

    #[test]
    fn tip5_hash_large_limbs_roundtrip() {
        let hash: Tip5Hash = [100, 200, 300, 400, 500];
        let entry = jam_tip5_entry("big", &hash);
        let data = NoteData::new(vec![entry]);
        assert_eq!(find_hash_entry(&data, "big").unwrap(), hash);
    }

    #[test]
    fn find_entry_missing_key() {
        let data = NoteData::new(vec![]);
        assert!(find_entry(&data, "nonexistent").is_err());
    }

    #[test]
    fn opaque_bytes_roundtrip() {
        let payload: Vec<u8> = (0..=255).collect();
        let entry = jam_opaque_bytes_entry("proof", &payload);
        let data = NoteData::new(vec![entry]);
        let decoded = find_opaque_bytes_entry(&data, "proof").unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn opaque_bytes_empty_roundtrip() {
        let entry = jam_opaque_bytes_entry("empty", &[]);
        let data = NoteData::new(vec![entry]);
        let decoded = find_opaque_bytes_entry(&data, "empty").unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn opaque_bytes_is_single_atom() {
        let payload: Vec<u8> = [0xDE, 0xAD, 0xBE, 0xEF].iter().copied().cycle().take(1024).collect();
        let entry = jam_opaque_bytes_entry("blob", &payload);
        // CUE the blob and verify it's an atom (1 leaf), not a cell tree
        let mut slab: NounSlab<NockJammer> = NounSlab::new();
        slab.cue_into(entry.blob.clone()).unwrap();
        let noun = slab_root(&slab);
        assert!(noun.is_atom(), "opaque bytes entry must CUE to a single atom");
    }

    #[test]
    fn multiple_entries() {
        let entries = vec![
            jam_u64_entry("version", 1),
            jam_u64_entry("id", 42),
            jam_tip5_entry("root", &[0xAA; 5]),
        ];
        let data = NoteData::new(entries);

        assert_eq!(find_u64_entry(&data, "version").unwrap(), 1);
        assert_eq!(find_u64_entry(&data, "id").unwrap(), 42);
        assert_eq!(find_hash_entry(&data, "root").unwrap(), [0xAA; 5]);
    }
}
