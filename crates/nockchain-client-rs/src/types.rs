//! Balance extraction types — SpendableUtxo and protobuf conversion helpers.
//!
//! Converts the protobuf `Balance` response from the chain's gRPC API into
//! Rust-friendly types. The main type is `SpendableUtxo` which carries the
//! note name, amount, and raw NoteData for app-specific decoding.

use nockchain_types::tx_engine::common::Hash as ChainHash;
use nockchain_types::tx_engine::v1::note::{NoteData, NoteDataEntry};

// ---------------------------------------------------------------------------
// SpendableUtxo
// ---------------------------------------------------------------------------

/// A spendable UTXO extracted from a protobuf Balance response.
///
/// Contains the fields needed to construct a transaction. The `note_data`
/// field carries the raw NoteData entries — callers decode these into their
/// app-specific types.
#[derive(Debug, Clone)]
pub struct SpendableUtxo {
    /// The note's first-name hash (used as input identifier).
    pub first_name: ChainHash,
    /// The note's last-name hash.
    pub last_name: ChainHash,
    /// The note's amount in nicks.
    pub amount: u64,
    /// Whether this is a NoteV1 (supports NoteData).
    pub is_v1: bool,
    /// Raw NoteData entries from the note (if NoteV1 with data).
    /// Callers decode these into app-specific types.
    pub note_data: Option<NoteData>,
}

impl SpendableUtxo {
    /// The note's first-name hash (base58 for gRPC queries).
    pub fn first_name(&self) -> &ChainHash {
        &self.first_name
    }

    /// The note's last-name hash.
    pub fn last_name(&self) -> &ChainHash {
        &self.last_name
    }
}

// ---------------------------------------------------------------------------
// Protobuf conversion helpers
// ---------------------------------------------------------------------------

/// Extract `NoteData` from a protobuf Note (v0 or v1 variant).
///
/// Only NoteV1 carries `note_data`; legacy v0 notes return `None`.
pub fn extract_note_data(note: &nockapp_grpc::pb::common::v2::Note) -> Option<NoteData> {
    use nockapp_grpc::pb::common::v2::note::NoteVersion;

    let variant = note.note_version.as_ref()?;
    match variant {
        NoteVersion::V1(v1) => {
            let pd = v1.note_data.as_ref()?;
            let entries: Vec<NoteDataEntry> = pd
                .entries
                .iter()
                .map(|e| NoteDataEntry::new(e.key.clone(), e.blob.clone().into()))
                .collect();
            if entries.is_empty() {
                None
            } else {
                Some(NoteData::new(entries))
            }
        }
        _ => None,
    }
}

/// Convert a protobuf Hash (5 belts) to a nockchain-types Hash.
pub fn chain_hash_from_pb(pb: &nockapp_grpc::pb::common::v1::Hash) -> ChainHash {
    ChainHash::from_limbs(&[
        pb.belt_1.as_ref().map_or(0, |b| b.value),
        pb.belt_2.as_ref().map_or(0, |b| b.value),
        pb.belt_3.as_ref().map_or(0, |b| b.value),
        pb.belt_4.as_ref().map_or(0, |b| b.value),
        pb.belt_5.as_ref().map_or(0, |b| b.value),
    ])
}

/// Extract spendable UTXO info from a protobuf Balance response.
///
/// Parses each `BalanceEntry` to extract the note name, amount, and
/// any NoteData. Skips entries with missing data.
pub fn extract_spendable_utxos(
    balance: &nockapp_grpc::pb::common::v2::Balance,
) -> Vec<SpendableUtxo> {
    let mut utxos = Vec::new();
    for entry in &balance.notes {
        let pb_name = match &entry.name {
            Some(n) => n,
            None => continue,
        };
        let note = match &entry.note {
            Some(n) => n,
            None => continue,
        };

        let first_name = match &pb_name.first {
            Some(h) => chain_hash_from_pb(h),
            None => continue,
        };
        let last_name = match &pb_name.last {
            Some(h) => chain_hash_from_pb(h),
            None => continue,
        };

        use nockapp_grpc::pb::common::v2::note::NoteVersion;
        let (is_v1, amount, note_data) = match &note.note_version {
            Some(NoteVersion::V1(v1)) => {
                let amt = match v1.assets.as_ref() {
                    Some(n) => n.value,
                    None => {
                        eprintln!("warn: V1 UTXO missing amount field, skipping");
                        continue;
                    }
                };
                let nd = extract_note_data(note);
                (true, amt, nd)
            }
            Some(NoteVersion::Legacy(v0)) => {
                let amt = match v0.assets.as_ref() {
                    Some(n) => n.value,
                    None => {
                        eprintln!("warn: legacy UTXO missing amount field, skipping");
                        continue;
                    }
                };
                (false, amt, None)
            }
            None => continue,
        };

        utxos.push(SpendableUtxo {
            first_name,
            last_name,
            amount,
            is_v1,
            note_data,
        });
    }
    utxos
}
