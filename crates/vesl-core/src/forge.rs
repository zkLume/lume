//! Forge — STARK Proof helpers (heaviest tier)
//!
//! Provides composable helpers for STARK proof handling:
//! - Extract proof bytes from kernel effects after a %prove poke
//! - Build a generic %prove poke from pre-jammed payload bytes
//!
//! The hull fires the %prove poke and manages the NockApp. Forge
//! provides the effect parsing and poke construction. Kernel boot
//! (including prover jet registration) is the hull's responsibility.

use anyhow::Result;
use nock_noun_rs::{atom_from_u64, make_atom_in, make_list_in, make_loobean, make_tag_in, jam_to_bytes, new_stack, NounSlab, T};
use nockchain_tip5_rs::tip5_to_atom_le_bytes;

use crate::types::ForgePayload;

/// Dereference a NounSlab's root noun (C-001).
///
/// Centralizes the unsafe dereference. The caller must ensure the slab's
/// root was populated (via `set_root`, `cue_into`, or `NockApp::poke`).
fn slab_root(slab: &NounSlab) -> nockvm::noun::Noun {
    unsafe { *slab.root() }
}

/// Extract proof bytes from kernel effects after a %prove poke.
///
/// Expected effect shape on success: `[result-note proof]`
///   where proof may be an atom or a cell (STARK proofs are structured).
/// Expected effect shape on failure: `[%prove-failed trace-jam]`
///
/// The proof noun is JAM'd to produce opaque bytes suitable for
/// on-chain storage or later verification.
///
/// Returns `Ok(Some(bytes))` if proof was extracted,
/// `Ok(None)` if the effect indicates failure (prove-failed),
/// `Err` if the effect structure is unexpected.
pub fn extract_proof_from_effects(effects: &[NounSlab]) -> Result<Option<bytes::Bytes>> {
    let effect_slab = match effects.first() {
        Some(slab) => slab,
        None => return Ok(None),
    };

    let root_noun = slab_root(effect_slab);

    let cell = match root_noun.as_cell() {
        Ok(c) => c,
        Err(_) => return Ok(None), // atom effect = no proof
    };

    // Check for failure tag: [%prove-failed ...]
    if let Ok(tag_atom) = cell.head().as_atom() {
        let tag_bytes = tag_atom.as_ne_bytes();
        if tag_bytes.starts_with(b"prove-failed") {
            return Ok(None);
        }
    }

    // Success: [result-note proof] where result-note is a cell.
    // The proof noun may be an atom or a cell — JAM it to get bytes.
    if cell.head().is_cell() {
        let proof_noun = cell.tail();
        let mut stack = new_stack();
        let proof_bytes = jam_to_bytes(&mut stack, proof_noun);
        if !proof_bytes.is_empty() {
            return Ok(Some(bytes::Bytes::from(proof_bytes)));
        }
    }

    Ok(None)
}

/// Build a generic `%prove` poke from pre-jammed payload bytes.
///
/// Constructs `[%prove jammed-payload-atom]`. The caller jams their
/// domain-specific payload before calling this.
pub fn build_prove_poke_generic(jammed_payload: &[u8]) -> NounSlab {
    let mut slab = NounSlab::new();
    let tag = make_atom_in(&mut slab, b"prove");
    let payload = make_atom_in(&mut slab, jammed_payload);
    let poke = T(&mut slab, &[tag, payload]);
    slab.set_root(poke);
    slab
}

/// Build a forge-payload noun inside a NounSlab.
///
/// Noun shape:
/// ```text
/// [note leaves expected-root]
///   note          = [id=@ hull=@ root=@ [%pending 0]]
///   leaves        = null-terminated list of [dat=@ proof=(list [hash=@ side=?])]
///   expected-root = @
/// ```
fn build_forge_payload_in(
    slab: &mut NounSlab,
    payload: &ForgePayload,
) -> nockvm::noun::Noun {
    // Note: [id=@ hull=@ root=@ state=[%pending ~]]
    // id/hull can exceed DIRECT_MAX — route through atom_from_u64.
    let id = atom_from_u64(slab, payload.note.id);
    let hull = atom_from_u64(slab, payload.note.hull);
    let root_bytes = tip5_to_atom_le_bytes(&payload.note.root);
    let root_noun = make_atom_in(slab, &root_bytes);
    let state_tag = make_tag_in(slab, "pending");
    let state = nockvm::noun::T(slab, &[state_tag, nockvm::noun::D(0)]);
    let note_noun = nockvm::noun::T(slab, &[id, hull, root_noun, state]);

    // Leaves: list of [dat=@ proof=(list [hash=@ side=?])]
    let leaf_nouns: Vec<nockvm::noun::Noun> = payload
        .leaves
        .iter()
        .map(|leaf| {
            let dat_atom = make_atom_in(slab, &leaf.dat);

            let proof_nodes: Vec<nockvm::noun::Noun> = leaf
                .proof
                .iter()
                .map(|p| {
                    let hash_bytes = tip5_to_atom_le_bytes(&p.hash);
                    let hash = make_atom_in(slab, &hash_bytes);
                    let side = make_loobean(p.side);
                    nockvm::noun::T(slab, &[hash, side])
                })
                .collect();
            let proof_list = make_list_in(slab, &proof_nodes);

            nockvm::noun::T(slab, &[dat_atom, proof_list])
        })
        .collect();
    let leaves_noun = make_list_in(slab, &leaf_nouns);

    // Expected root
    let exp_root_bytes = tip5_to_atom_le_bytes(&payload.expected_root);
    let exp_root = make_atom_in(slab, &exp_root_bytes);

    nockvm::noun::T(slab, &[note_noun, leaves_noun, exp_root])
}

/// Build a forge poke with the given tag and payload.
fn build_forge_poke(tag: &str, payload: &ForgePayload) -> NounSlab {
    let mut slab = NounSlab::new();

    let tag_noun = make_tag_in(&mut slab, tag);
    let payload_noun = build_forge_payload_in(&mut slab, payload);
    let payload_bytes = {
        let mut stack = new_stack();
        jam_to_bytes(&mut stack, payload_noun)
    };
    let jammed = make_atom_in(&mut slab, &payload_bytes);

    let poke = nockvm::noun::T(&mut slab, &[tag_noun, jammed]);
    slab.set_root(poke);
    slab
}

/// Build a `[%prove jammed-forge-payload]` poke.
pub fn build_forge_prove_poke(payload: &ForgePayload) -> NounSlab {
    build_forge_poke("prove", payload)
}

/// Build a `[%settle jammed-forge-payload]` poke.
pub fn build_forge_settle_poke(payload: &ForgePayload) -> NounSlab {
    build_forge_poke("settle", payload)
}

/// Build a `[%verify jammed-forge-payload]` poke.
pub fn build_forge_verify_poke(payload: &ForgePayload) -> NounSlab {
    build_forge_poke("verify", payload)
}

/// Placeholder struct for future full STARK prover integration.
///
/// When the STARK prover is wired in, this struct will hold the
/// hot state (zkvm-jetpack prover context). For now, the hull
/// handles kernel boot with prover jets directly.
pub struct Forge;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nock_noun_rs::{cue_from_bytes, new_stack, D};
    use crate::types::{LeafWithProof, Note, NoteState};
    use nockchain_tip5_rs::{ProofNode, TIP5_ZERO};

    fn test_payload() -> ForgePayload {
        ForgePayload {
            note: Note {
                id: 1,
                hull: 42,
                root: TIP5_ZERO,
                state: NoteState::Pending,
            },
            leaves: vec![
                LeafWithProof {
                    dat: vec![0xDE, 0xAD],
                    proof: vec![],
                },
            ],
            expected_root: TIP5_ZERO,
        }
    }

    fn multi_leaf_payload() -> ForgePayload {
        ForgePayload {
            note: Note {
                id: 7,
                hull: 99,
                root: [1, 2, 3, 4, 5],
                state: NoteState::Pending,
            },
            leaves: vec![
                LeafWithProof {
                    dat: b"first leaf data".to_vec(),
                    proof: vec![
                        ProofNode { hash: [10, 20, 30, 40, 50], side: true },
                    ],
                },
                LeafWithProof {
                    dat: b"second leaf data".to_vec(),
                    proof: vec![
                        ProofNode { hash: [50, 40, 30, 20, 10], side: false },
                        ProofNode { hash: [5, 5, 5, 5, 5], side: true },
                    ],
                },
                LeafWithProof {
                    dat: b"third leaf".to_vec(),
                    proof: vec![],
                },
            ],
            expected_root: [1, 2, 3, 4, 5],
        }
    }

    // --- Basic poke construction ---

    #[test]
    fn build_prove_poke_generic_produces_cell() {
        let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let slab = build_prove_poke_generic(&payload);
        let root = slab_root(&slab);
        assert!(root.is_cell(), "prove poke must be a cell [%prove payload]");
    }

    #[test]
    fn build_forge_prove_poke_produces_cell() {
        let slab = build_forge_prove_poke(&test_payload());
        let root = slab_root(&slab);
        assert!(root.is_cell(), "forge prove poke must be a cell");
    }

    #[test]
    fn build_forge_settle_poke_produces_cell() {
        let slab = build_forge_settle_poke(&test_payload());
        let root = slab_root(&slab);
        assert!(root.is_cell(), "forge settle poke must be a cell");
    }

    #[test]
    fn build_forge_verify_poke_produces_cell() {
        let slab = build_forge_verify_poke(&test_payload());
        let root = slab_root(&slab);
        assert!(root.is_cell(), "forge verify poke must be a cell");
    }

    // --- Multi-leaf payloads ---

    #[test]
    fn build_forge_prove_poke_multi_leaf() {
        let slab = build_forge_prove_poke(&multi_leaf_payload());
        let root = slab_root(&slab);
        assert!(root.is_cell(), "multi-leaf prove poke must be a cell");
    }

    #[test]
    fn build_forge_settle_poke_multi_leaf() {
        let slab = build_forge_settle_poke(&multi_leaf_payload());
        let root = slab_root(&slab);
        assert!(root.is_cell(), "multi-leaf settle poke must be a cell");
    }

    // --- Tag verification (poke head is the correct tag atom) ---

    #[test]
    fn forge_poke_tags_are_correct() {
        let payload = test_payload();

        for (builder, expected_tag) in [
            (build_forge_prove_poke as fn(&ForgePayload) -> NounSlab, "prove"),
            (build_forge_settle_poke, "settle"),
            (build_forge_verify_poke, "verify"),
        ] {
            let slab = builder(&payload);
            let root = slab_root(&slab);
            let cell = root.as_cell().expect("poke is a cell");
            let tag_atom = cell.head().as_atom().expect("head is a tag atom");
            let tag_bytes = tag_atom.as_ne_bytes();
            // Trim trailing null bytes for comparison
            let len = tag_bytes.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
            let tag_str = std::str::from_utf8(&tag_bytes[..len]).expect("tag is valid utf8");
            assert_eq!(tag_str, expected_tag, "poke tag mismatch");
        }
    }

    // --- Jam/Cue roundtrip: the jammed payload inside the poke can be CUE'd
    //     back to a valid noun structure matching forge-payload shape ---

    #[test]
    fn forge_payload_jam_cue_roundtrip() {
        let payload = multi_leaf_payload();
        let slab = build_forge_prove_poke(&payload);
        let root = slab_root(&slab);

        // root = [%prove jammed-payload-atom]
        let cell = root.as_cell().expect("poke is a cell");
        let jammed_atom = cell.tail().as_atom().expect("tail is jammed payload atom");
        let jammed_bytes = jammed_atom.as_ne_bytes();
        let len = jammed_bytes.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);

        // CUE the jammed payload back to a noun
        let mut stack = new_stack();
        let cued = cue_from_bytes(&mut stack, &jammed_bytes[..len])
            .expect("cue must succeed on jammed forge-payload");

        // forge-payload = [note leaves expected-root]
        // note = [id=@ hull=@ root=@ state=[%pending 0]]
        assert!(cued.is_cell(), "cued payload must be a cell");
        let outer = cued.as_cell().unwrap();

        // Head is note (a cell)
        let note_noun = outer.head();
        assert!(note_noun.is_cell(), "note must be a cell [id hull root state]");

        // Verify note.id
        let note_cell = note_noun.as_cell().unwrap();
        let id_atom = note_cell.head().as_atom().expect("note.id is an atom");
        assert_eq!(id_atom.as_u64().unwrap(), 7, "note.id should be 7");

        // rest = [leaves expected-root]
        let rest = outer.tail();
        assert!(rest.is_cell(), "rest [leaves expected-root] must be a cell");
        let rest_cell = rest.as_cell().unwrap();

        // leaves is a list (cell or 0 for empty)
        let leaves_noun = rest_cell.head();
        assert!(leaves_noun.is_cell(), "leaves list with 3 items must be a cell");

        // expected-root is an atom
        let exp_root = rest_cell.tail();
        assert!(exp_root.is_atom(), "expected-root must be an atom");
    }

    // --- Determinism: same payload produces identical jammed bytes ---

    #[test]
    fn forge_poke_is_deterministic() {
        let payload = multi_leaf_payload();
        let slab_1 = build_forge_prove_poke(&payload);
        let slab_2 = build_forge_prove_poke(&payload);

        let mut stack = new_stack();
        let bytes_1 = jam_to_bytes(&mut stack, slab_root(&slab_1));
        let bytes_2 = jam_to_bytes(&mut stack, slab_root(&slab_2));
        assert_eq!(bytes_1, bytes_2, "same payload must produce identical jam output");
    }

    // --- Effect extraction ---

    #[test]
    fn extract_proof_from_empty_effects() {
        let effects: Vec<NounSlab> = vec![];
        let result = extract_proof_from_effects(&effects).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn extract_proof_from_cell_with_proof() {
        // Simulate: [result-note proof-atom]
        // result-note is a cell [42 7], proof is an atom
        let mut slab = NounSlab::new();
        let note = T(&mut slab, &[D(42), D(7)]);
        let proof_data = vec![0xCA, 0xFE, 0xBA, 0xBE];
        let proof = make_atom_in(&mut slab, &proof_data);
        let effect = T(&mut slab, &[note, proof]);
        slab.set_root(effect);

        let result = extract_proof_from_effects(&[slab]).unwrap();
        assert!(result.is_some(), "must extract proof bytes");
        let bytes = result.unwrap();
        // Extracted bytes are JAM'd proof noun — CUE back to verify
        let mut stack = new_stack();
        let cued = cue_from_bytes(&mut stack, &bytes).expect("JAM'd proof must CUE");
        let recovered = cued.as_atom().expect("original was an atom");
        let orig_bytes = recovered.as_ne_bytes();
        let len = orig_bytes.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
        assert_eq!(&orig_bytes[..len], &[0xCA, 0xFE, 0xBA, 0xBE]);
    }

    #[test]
    fn extract_proof_from_prove_failed_effect() {
        // Simulate: [%prove-failed jammed-trace]
        let mut slab = NounSlab::new();
        let tag = make_atom_in(&mut slab, b"prove-failed");
        let trace = make_atom_in(&mut slab, b"some-trace-data");
        let effect = T(&mut slab, &[tag, trace]);
        slab.set_root(effect);

        let result = extract_proof_from_effects(&[slab]).unwrap();
        assert!(result.is_none(), "prove-failed effect should return None");
    }

    // --- Effect shape parity with forge-kernel.hoon ---
    //
    // forge-kernel.hoon %prove success emits:
    //   [result-note proof] where result-note = [id hull root [%settled ~]]
    //
    // extract_proof_from_effects checks cell.head().is_cell() — the note
    // cell IS a cell, so it enters the success path. Verify this.

    #[test]
    fn extract_proof_from_forge_kernel_effect_shape() {
        let mut slab = NounSlab::new();

        // Build result-note: [id=1 hull=42 root=7 [%settled 0]]
        let settled_tag = make_tag_in(&mut slab, "settled");
        let state = nockvm::noun::T(&mut slab, &[settled_tag, D(0)]);
        let result_note = nockvm::noun::T(&mut slab, &[D(1), D(42), D(7), state]);

        // Proof atom
        let proof_bytes = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let proof_atom = make_atom_in(&mut slab, &proof_bytes);

        // Effect: [result-note proof-atom]
        let effect = nockvm::noun::T(&mut slab, &[result_note, proof_atom]);
        slab.set_root(effect);

        let result = extract_proof_from_effects(&[slab]).unwrap();
        assert!(result.is_some(), "forge kernel success effect must extract proof");
        let bytes = result.unwrap();
        // JAM'd proof — CUE back to verify original atom
        let mut stack = new_stack();
        let cued = cue_from_bytes(&mut stack, &bytes).expect("JAM'd proof must CUE");
        let recovered = cued.as_atom().expect("original was an atom");
        let orig = recovered.as_ne_bytes();
        let len = orig.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
        assert_eq!(&orig[..len], &[0x01, 0x02, 0x03, 0x04, 0x05]);
    }
}
