//! Raw transaction builder for Vesl settlement on Nockchain.
//!
//! Phase 3.5.2b: Constructs and signs raw transactions in Rust with custom
//! NoteData, using the Hoon kernel's tx-engine hashable infrastructure for
//! byte-exact sig-hash and tx-id computation.
//!
//! The sig-hash and tx-id are computed via two kernel pokes (%sig-hash, %tx-id)
//! rather than reimplementing Hoon's ~300-line recursive hashable tree in Rust.

use nockapp::noun::slab::{NockJammer, NounSlab};
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use nockchain_math::belt::Belt;
use nockchain_types::tx_engine::common::{Hash, Name, Nicks};
use nockchain_types::tx_engine::v1::note::NoteData;
use nockchain_types::tx_engine::v1::tx::{
    Lock, LockMerkleProof, LockMerkleProofFull, MerkleProof, PkhSignature, PkhSignatureEntry,
    Seed, Seeds, Spend, Spend1, Spends, SpendCondition, Witness,
};
use nockchain_types::tx_engine::v1::{RawTx, Version};
use nockvm::ext::make_tas;
use nockvm::noun::{IndirectAtom, D, T};
use nockvm_macros::tas;
use noun_serde::{NounDecode, NounEncode};

use crate::chain::SettlementData;
use crate::signing;

// ---------------------------------------------------------------------------
// Settlement transaction builder
// ---------------------------------------------------------------------------

/// Parameters for building a settlement transaction.
pub struct SettlementTxParams {
    /// The coinbase UTXO to spend (note name).
    pub input_name: Name,
    /// The input note's hash (parent-hash for the output seed).
    pub input_note_hash: Hash,
    /// The input note's amount.
    pub input_amount: u64,
    /// Whether the input is a coinbase note.
    pub is_coinbase: bool,
    /// Coinbase relative timelock minimum (only used if `is_coinbase` is true).
    /// Fakenet default is 1.
    pub coinbase_timelock_min: u64,
    /// The input note's source hash (tx hash or block hash for coinbase).
    pub source_hash: Hash,
    /// The recipient's PKH (public key hash) — typically self-send.
    pub recipient_pkh: Hash,
    /// The Vesl settlement data to embed.
    pub settlement: SettlementData,
    /// Transaction fee.
    pub fee: u64,
    /// The signing secret key (8 × 32-bit Belt chunks).
    pub signing_key: [Belt; 8],
}

/// Build a signed settlement transaction with Vesl NoteData.
///
/// Uses the Hoon kernel for sig-hash and tx-id computation (two pokes):
/// 1. `%sig-hash` — compute signing message from seeds + fee
/// 2. `%tx-id` — compute transaction ID from complete spends
pub async fn build_settlement_tx(
    app: &mut NockApp,
    params: &SettlementTxParams,
) -> anyhow::Result<RawTx> {
    // 1. Derive signing pubkey and PKH
    let pubkey = signing::derive_pubkey(&params.signing_key);
    let pkh = signing::pubkey_hash(&pubkey);

    // 2. Build OUTPUT lock (simple P2PKH for recipient — settlement output)
    let recipient_condition = SpendCondition::simple_pkh(params.recipient_pkh.clone());
    let output_lock_root = Lock::SpendCondition(recipient_condition.clone())
        .hash()
        .map_err(|e| anyhow::anyhow!("output lock hash failed: {e}"))?;

    // 3. Build INPUT lock (must match the UTXO being spent)
    //    Coinbase UTXOs have a P2PKH + timelock lock; regular UTXOs have simple P2PKH.
    let input_condition = if params.is_coinbase {
        SpendCondition::coinbase_pkh(pkh.clone(), params.coinbase_timelock_min)
    } else {
        SpendCondition::simple_pkh(pkh.clone())
    };
    let input_lock_root = Lock::SpendCondition(input_condition.clone())
        .hash()
        .map_err(|e| anyhow::anyhow!("input lock hash failed: {e}"))?;

    // 4. Encode Vesl settlement as NoteData entries
    let note_data = settlement_to_note_data(&params.settlement);

    // 5. Build the output seed
    let output_amount = params.input_amount.saturating_sub(params.fee);
    let seed = Seed {
        output_source: None, // Not a coinbase output
        lock_root: output_lock_root,
        note_data,
        gift: Nicks(output_amount as usize),
        parent_hash: params.input_note_hash.clone(),
    };

    let seeds = Seeds(vec![seed.clone()]);
    let fee = Nicks(params.fee as usize);

    // 6. Compute sig-hash via Hoon kernel
    let sig_hash = kernel_sig_hash(app, &seeds, &fee).await?;

    // 7. Sign the sig-hash
    let msg_belts = sig_hash.to_array().map(Belt);
    let signature = signing::sign(&params.signing_key, &msg_belts);

    // 8. Build witness — proves authorization to spend the INPUT UTXO
    let lock_merkle_proof = LockMerkleProofFull {
        version: tas!(b"full"),
        spend_condition: input_condition,
        axis: 1, // Root axis — single lock primitive
        proof: MerkleProof {
            root: input_lock_root,
            path: vec![],
        },
    };

    let pkh_sig_entry = PkhSignatureEntry {
        hash: pkh,
        pubkey,
        signature,
    };

    let witness = Witness::new(
        LockMerkleProof::Full(lock_merkle_proof),
        PkhSignature::new(vec![pkh_sig_entry]),
        vec![], // no hax preimages
    );

    // 9. Build spend
    let spend = Spend::Witness(Spend1 {
        witness,
        seeds,
        fee,
    });

    let spends = Spends(vec![(params.input_name.clone(), spend)]);

    // 10. Compute transaction ID via Hoon kernel
    let tx_id = kernel_tx_id(app, &spends).await?;

    // 11. Assemble transaction
    Ok(RawTx {
        version: Version::V1,
        id: tx_id,
        spends,
    })
}

// ---------------------------------------------------------------------------
// NoteData encoding for Vesl settlement
// ---------------------------------------------------------------------------

/// Encode Vesl SettlementData as NoteData entries (JAM'd nouns).
pub fn settlement_to_note_data(settlement: &SettlementData) -> NoteData {
    settlement.to_note_data()
}

// ---------------------------------------------------------------------------
// Kernel-based hash computation
// ---------------------------------------------------------------------------

/// Compute sig-hash by poking the Hoon kernel's `%sig-hash` handler.
///
/// Sends `[%sig-hash seeds-jam fee]` where `seeds-jam` is the JAM'd noun
/// of the Seeds z-set. Returns the tip5 hash used as the signing message.
pub async fn kernel_sig_hash(
    app: &mut NockApp,
    seeds: &Seeds,
    fee: &Nicks,
) -> anyhow::Result<Hash> {
    // JAM the seeds noun — use manual z-set construction to avoid
    // upstream NockStack issue with NoteData::to_noun() in ZSet context.
    let seeds_jammed = jam_seeds_manual(seeds)?;

    // Build the poke: [%sig-hash seeds-jam fee]
    let mut poke_slab: NounSlab = NounSlab::new();
    let tag = make_tas(&mut poke_slab, "sig-hash").as_noun();
    let seeds_atom = bytes_to_atom(&mut poke_slab, &seeds_jammed);
    let fee_noun = D(fee.0 as u64);
    let cmd = T(&mut poke_slab, &[tag, seeds_atom, fee_noun]);
    poke_slab.set_root(cmd);

    // Poke the kernel
    let effects = app
        .poke(SystemWire.to_wire(), poke_slab)
        .await
        .map_err(|e| anyhow::anyhow!("sig-hash poke failed: {e:?}"))?;

    // Extract the hash from the first effect: [%sig-hash hash]
    extract_hash_from_effect(&effects, "sig-hash")
}

/// Compute tx-id by poking the Hoon kernel's `%tx-id` handler.
///
/// Sends `[%tx-id spends-jam]` where `spends-jam` is the JAM'd noun
/// of the Spends z-map (including witness with real signatures).
pub async fn kernel_tx_id(
    app: &mut NockApp,
    spends: &Spends,
) -> anyhow::Result<Hash> {
    // JAM the spends noun — use manual z-map construction to avoid
    // upstream NockStack issue with NoteData::to_noun() in ZSet context.
    let spends_jammed = jam_spends_manual(spends)?;

    // Build the poke: [%tx-id spends-jam]
    let mut poke_slab: NounSlab = NounSlab::new();
    let tag = make_tas(&mut poke_slab, "tx-id").as_noun();
    let spends_atom = bytes_to_atom(&mut poke_slab, &spends_jammed);
    let cmd = T(&mut poke_slab, &[tag, spends_atom]);
    poke_slab.set_root(cmd);

    // Poke the kernel
    let effects = app
        .poke(SystemWire.to_wire(), poke_slab)
        .await
        .map_err(|e| anyhow::anyhow!("tx-id poke failed: {e:?}"))?;

    // Extract the hash from the first effect: [%tx-id hash]
    extract_hash_from_effect(&effects, "tx-id")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Manual noun builders — work around NockStack issue in ZSet/z-map
// ---------------------------------------------------------------------------

/// JAM Seeds into a noun on a plain NounSlab, bypassing ZSet::try_from_items
/// which creates an internal NockStack that fails with NoteData::to_noun().
///
/// For a single-seed z-set, the noun structure is `[seed 0 0]`
/// (treap node with null children).
fn jam_seeds_manual(seeds: &Seeds) -> anyhow::Result<bytes::Bytes> {
    anyhow::ensure!(!seeds.0.is_empty(), "seeds must not be empty");
    anyhow::ensure!(
        seeds.0.len() == 1,
        "manual seeds JAM only supports single-seed (have {})",
        seeds.0.len()
    );

    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let seed_noun = seeds.0[0].to_noun(&mut slab);
    // Single-element z-set: [element null null]
    let zset_noun = T(&mut slab, &[seed_noun, D(0), D(0)]);
    slab.set_root(zset_noun);
    Ok(slab.jam())
}

/// JAM Spends into a noun on a plain NounSlab, bypassing the ZMap machinery.
///
/// For a single-spend z-map, the noun structure is `[[key value] 0 0]`
/// (treap node with null children).
fn jam_spends_manual(spends: &Spends) -> anyhow::Result<bytes::Bytes> {
    anyhow::ensure!(!spends.0.is_empty(), "spends must not be empty");
    anyhow::ensure!(
        spends.0.len() == 1,
        "manual spends JAM only supports single-spend (have {})",
        spends.0.len()
    );

    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let (ref name, ref spend) = spends.0[0];
    let name_noun = name.to_noun(&mut slab);
    let spend_noun = spend.to_noun(&mut slab);
    let kv = T(&mut slab, &[name_noun, spend_noun]);
    // Single-element z-map: [kv null null]
    let zmap_noun = T(&mut slab, &[kv, D(0), D(0)]);
    slab.set_root(zmap_noun);
    Ok(slab.jam())
}

/// Extract a Hash from a kernel effect of shape `[%tag hash-noun]`.
fn extract_hash_from_effect(effects: &[NounSlab], expected_tag: &str) -> anyhow::Result<Hash> {
    let effect_slab = effects
        .first()
        .ok_or_else(|| anyhow::anyhow!("no effects returned from %{expected_tag} poke"))?;

    let root = unsafe { *effect_slab.root() };
    let cell = root
        .as_cell()
        .map_err(|_| anyhow::anyhow!("{expected_tag} effect is not a cell"))?;

    // The hash is the tail (head is the tag atom)
    let hash_noun = cell.tail();
    Hash::from_noun(&hash_noun).map_err(|e| anyhow::anyhow!("{expected_tag} hash decode: {e}"))
}

/// Convert a byte slice (JAM'd output) to a Nock atom.
fn bytes_to_atom(slab: &mut NounSlab, bytes: &[u8]) -> nockvm::noun::Noun {
    if bytes.is_empty() {
        return D(0);
    }
    unsafe {
        let mut indirect = IndirectAtom::new_raw_bytes_ref(slab, bytes);
        indirect.normalize_as_atom().as_noun()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nockchain_types::tx_engine::common::Hash;

    #[test]
    fn settlement_note_data_encodes() {
        let settlement = SettlementData {
            version: 1,
            hull_id: 7,
            merkle_root: [1, 2, 3, 4, 5],
            note_id: 42,
            manifest_hash: [6, 7, 8, 9, 10],
        };
        let nd = settlement_to_note_data(&settlement);
        assert_eq!(nd.0.len(), 5);
    }

    /// A1 diagnostic: Compare `jam_seeds_manual` output with `Seeds::to_noun` → JAM.
    ///
    /// If these differ, the sig-hash computed by our kernel differs from what
    /// the chain computes, causing `v1-spend-1-lock-failed` (ISSUE-006).
    #[test]
    fn jam_seeds_manual_matches_seeds_to_noun() {
        use noun_serde::NounEncode;

        // Build a Seed with Vesl NoteData — same structure as settlement path
        let settlement = SettlementData {
            version: 1,
            hull_id: 7,
            merkle_root: [100, 200, 300, 400, 500],
            note_id: 42,
            manifest_hash: [10, 20, 30, 40, 50],
        };
        let note_data = settlement_to_note_data(&settlement);

        let seed = Seed {
            output_source: None,
            lock_root: Hash::from_limbs(&[1, 2, 3, 4, 5]),
            note_data,
            gift: Nicks(62_536),
            parent_hash: Hash::from_limbs(&[10, 20, 30, 40, 50]),
        };
        let seeds = Seeds(vec![seed]);

        // Path 1: manual JAM (what we use for sig-hash)
        let manual_jam = jam_seeds_manual(&seeds).expect("manual JAM should succeed");

        // Path 2: Seeds::to_noun → JAM (what the chain uses after protobuf roundtrip)
        let standard_jam = {
            let mut slab: NounSlab<NockJammer> = NounSlab::new();
            let noun = seeds.to_noun(&mut slab);
            slab.set_root(noun);
            slab.jam()
        };

        // Compare
        let manual_bytes = manual_jam.to_vec();
        let standard_bytes = standard_jam.to_vec();

        if manual_bytes != standard_bytes {
            eprintln!("ISSUE-006 ROOT CAUSE FOUND: JAM divergence!");
            eprintln!(
                "  manual JAM:   {} bytes, first 32: {:?}",
                manual_bytes.len(),
                &manual_bytes[..manual_bytes.len().min(32)]
            );
            eprintln!(
                "  standard JAM: {} bytes, first 32: {:?}",
                standard_bytes.len(),
                &standard_bytes[..standard_bytes.len().min(32)]
            );
        }

        assert_eq!(
            manual_bytes, standard_bytes,
            "jam_seeds_manual must produce identical bytes to Seeds::to_noun → JAM. \
             Divergence causes sig-hash mismatch → v1-spend-1-lock-failed (ISSUE-006)."
        );
    }

    /// A1b diagnostic: Same comparison for `jam_spends_manual` vs `Spends::to_noun`.
    #[test]
    fn jam_spends_manual_matches_spends_to_noun() {
        use noun_serde::NounEncode;

        let settlement = SettlementData {
            version: 1,
            hull_id: 7,
            merkle_root: [100, 200, 300, 400, 500],
            note_id: 42,
            manifest_hash: [10, 20, 30, 40, 50],
        };
        let note_data = settlement_to_note_data(&settlement);

        let seed = Seed {
            output_source: None,
            lock_root: Hash::from_limbs(&[1, 2, 3, 4, 5]),
            note_data,
            gift: Nicks(62_536),
            parent_hash: Hash::from_limbs(&[10, 20, 30, 40, 50]),
        };
        let seeds = Seeds(vec![seed]);
        let fee = Nicks(3000);

        // Build a minimal Witness for the Spend
        let pkh = Hash::from_limbs(&[99, 88, 77, 66, 55]);
        let input_condition = SpendCondition::coinbase_pkh(pkh.clone(), 1);
        let input_lock_root = Lock::SpendCondition(input_condition.clone())
            .hash()
            .expect("lock hash");

        let lock_merkle_proof = LockMerkleProofFull {
            version: nockvm_macros::tas!(b"full"),
            spend_condition: input_condition,
            axis: 1,
            proof: MerkleProof {
                root: input_lock_root,
                path: vec![],
            },
        };

        // Use a dummy signature (won't affect JAM comparison)
        let dummy_sig = nockchain_types::tx_engine::common::SchnorrSignature {
            chal: [nockchain_math::belt::Belt(1); 8],
            sig: [nockchain_math::belt::Belt(2); 8],
        };
        let dummy_pk = crate::signing::derive_pubkey(&crate::signing::demo_signing_key());

        let pkh_sig_entry = PkhSignatureEntry {
            hash: pkh,
            pubkey: dummy_pk,
            signature: dummy_sig,
        };

        let witness = Witness::new(
            LockMerkleProof::Full(lock_merkle_proof),
            PkhSignature::new(vec![pkh_sig_entry]),
            vec![],
        );

        let spend = Spend::Witness(Spend1 {
            witness,
            seeds,
            fee,
        });

        let name = nockchain_types::tx_engine::common::Name::new(
            Hash::from_limbs(&[1, 1, 1, 1, 1]),
            Hash::from_limbs(&[2, 2, 2, 2, 2]),
        );
        let spends = Spends(vec![(name, spend)]);

        // Path 1: manual JAM
        let manual_jam = jam_spends_manual(&spends).expect("manual JAM should succeed");

        // Path 2: Spends::to_noun → JAM
        let standard_jam = {
            let mut slab: NounSlab<NockJammer> = NounSlab::new();
            let noun = spends.to_noun(&mut slab);
            slab.set_root(noun);
            slab.jam()
        };

        let manual_bytes = manual_jam.to_vec();
        let standard_bytes = standard_jam.to_vec();

        if manual_bytes != standard_bytes {
            eprintln!("ISSUE-006 ROOT CAUSE FOUND: Spends JAM divergence!");
            eprintln!("  manual:   {} bytes", manual_bytes.len());
            eprintln!("  standard: {} bytes", standard_bytes.len());
        }

        assert_eq!(
            manual_bytes, standard_bytes,
            "jam_spends_manual must produce identical bytes to Spends::to_noun → JAM."
        );
    }
}
