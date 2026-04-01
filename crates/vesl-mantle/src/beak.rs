//! Beak — Settlement (heavy tentacle)
//!
//! Full settlement lifecycle. Requires kernel boot + chain access.
//! Wraps Grip verification with NockApp kernel state transitions
//! and on-chain submission via nockchain-client-rs.

use anyhow::Result;

use nock_noun_rs::NounSlab;
use nockchain_client_rs::{ChainClient, WalletClient};
use nockchain_tip5_rs::Tip5Hash;

use crate::grip::Grip;
use crate::types::{Manifest, Note};

pub struct Beak {
    grip: Grip,
    // Kernel handle will go here when boot is implemented.
    // For now, Beak provides the API surface with stubs.
}

impl Beak {
    /// Boot the Vesl kernel for settlement.
    ///
    /// Loads beak.jam (or kraken.jam) and initializes the NockApp.
    /// Currently stubbed — kernel boot wiring depends on runtime environment.
    pub async fn boot() -> Result<Self> {
        todo!("kernel boot: load beak.jam, init NockApp runtime")
    }

    /// Create a Beak with just the Grip verifier (no kernel).
    /// Useful for testing the verification path without kernel boot.
    pub fn without_kernel() -> Self {
        Beak { grip: Grip::new() }
    }

    /// Register a root as trusted in the local verifier.
    pub fn register_root(&mut self, root: Tip5Hash) {
        self.grip.register_root(root);
    }

    /// Settle a manifest: verify + state transition via kernel poke.
    ///
    /// Verification happens locally via Grip. The kernel poke
    /// transitions the note from %pending to %settled.
    /// Returns the settled note.
    pub async fn settle(
        &mut self,
        note: &Note,
        manifest: &Manifest,
        root: &Tip5Hash,
    ) -> Result<Note> {
        // Local verification first
        anyhow::ensure!(
            self.grip.check_manifest(manifest, root),
            "manifest verification failed"
        );

        // Build kernel poke (reuses hull noun_builder patterns)
        let _poke: NounSlab = build_settle_poke(note, manifest, root);

        // TODO: send poke to kernel, get settled note back
        // For now, return the note transitioned to Settled
        Ok(Note {
            id: note.id,
            hull: note.hull,
            root: note.root,
            state: crate::types::NoteState::Settled,
        })
    }

    /// Settle and submit to chain.
    ///
    /// Requires a connected ChainClient and WalletClient.
    /// Currently stubbed — chain submission wiring is complex.
    pub async fn settle_on_chain(
        &mut self,
        _note: &Note,
        _manifest: &Manifest,
        _root: &Tip5Hash,
        _chain: &mut ChainClient,
        _wallet: &mut WalletClient,
    ) -> Result<()> {
        todo!("settle_on_chain: verify → kernel poke → build tx → sign → submit")
    }

    /// Access the inner Grip verifier.
    pub fn grip(&self) -> &Grip {
        &self.grip
    }
}

/// Build a [%settle jammed-payload] poke in NounSlab.
///
/// Mirrors hull/src/noun_builder.rs build_settle_poke.
/// Public for cross-runtime alignment testing.
pub fn build_settle_poke(
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> NounSlab {
    use nock_noun_rs::*;

    let mut slab = NounSlab::new();

    let tag = make_tag_in(&mut slab, "settle");
    let payload = build_settlement_payload_in(&mut slab, note, manifest, expected_root);
    let payload_bytes = {
        let mut stack = new_stack();
        jam_to_bytes(&mut stack, payload)
    };
    let jammed = make_atom_in(&mut slab, &payload_bytes);

    let poke = nockvm::noun::T(&mut slab, &[tag, jammed]);
    slab.set_root(poke);
    slab
}

/// Build a [%register hull=@ root=@] poke in NounSlab.
///
/// Mirrors hull/src/noun_builder.rs build_register_poke.
/// Public for cross-runtime alignment testing.
pub fn build_register_poke(hull_id: u64, root: &Tip5Hash) -> NounSlab {
    use nock_noun_rs::*;
    use nockchain_tip5_rs::tip5_to_atom_le_bytes;

    let mut slab = NounSlab::new();

    let tag = make_tag_in(&mut slab, "register");
    let hull = nockvm::noun::D(hull_id);
    let root_bytes = tip5_to_atom_le_bytes(root);
    let root_noun = make_atom_in(&mut slab, &root_bytes);

    let poke = nockvm::noun::T(&mut slab, &[tag, hull, root_noun]);
    slab.set_root(poke);
    slab
}

/// Build settlement payload noun in a NounSlab.
///
/// Encodes note + manifest + root as nested noun structure matching
/// the Hoon settlement-payload type.
fn build_settlement_payload_in(
    slab: &mut NounSlab,
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> nockvm::noun::Noun {
    use nock_noun_rs::*;
    use nockchain_tip5_rs::tip5_to_atom_le_bytes;

    // Note: [id=@ hull=@ root=@ state=[%pending ~]]
    let id = nockvm::noun::D(note.id);
    let hull = nockvm::noun::D(note.hull);
    let root_bytes = tip5_to_atom_le_bytes(&note.root);
    let root_noun = make_atom_in(slab, &root_bytes);
    let state_tag = make_tag_in(slab, "pending");
    let state = nockvm::noun::T(slab, &[state_tag, nockvm::noun::D(0)]);
    let note_noun = nockvm::noun::T(slab, &[id, hull, root_noun, state]);

    // Manifest: [query results prompt output]
    let query = make_cord_in(slab, &manifest.query);
    let prompt = make_cord_in(slab, &manifest.prompt);
    let output = make_cord_in(slab, &manifest.output);

    let results: Vec<nockvm::noun::Noun> = manifest
        .results
        .iter()
        .map(|r| {
            let chunk_id = nockvm::noun::D(r.chunk.id);
            let chunk_dat = make_cord_in(slab, &r.chunk.dat);
            let chunk = nockvm::noun::T(slab, &[chunk_id, chunk_dat]);

            let proof_nodes: Vec<nockvm::noun::Noun> = r
                .proof
                .iter()
                .map(|p| {
                    let hash_bytes = tip5_to_atom_le_bytes(&p.hash);
                    let hash = make_atom_in(slab, &hash_bytes);
                    let side = make_loobean(p.side);
                    nockvm::noun::T(slab, &[hash, side])
                })
                .collect();
            let proof = make_list_in(slab, &proof_nodes);

            let score = nockvm::noun::D(r.score);
            nockvm::noun::T(slab, &[chunk, proof, score])
        })
        .collect();
    let results_noun = make_list_in(slab, &results);

    let manifest_noun = nockvm::noun::T(slab, &[query, results_noun, prompt, output]);

    // Expected root
    let exp_root_bytes = tip5_to_atom_le_bytes(expected_root);
    let exp_root = make_atom_in(slab, &exp_root_bytes);

    nockvm::noun::T(slab, &[note_noun, manifest_noun, exp_root])
}
