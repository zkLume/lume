//! Chain integration — maps Lume settlement data to Nockchain's transaction model.
//!
//! Phases 3.1 + 3.2 of the DEV.md roadmap. Implements **Strategy A: Data-in-Note**.
//!
//! # Strategy A
//!
//! Lume settlement data (vessel ID, Merkle root, manifest hash) is embedded
//! in the `note_data` field of a standard Nockchain `NoteV1` / `Seed`.
//! The on-chain guarantee is: "a Note exists with this data, signed by
//! Lume's key." The chain itself does not enforce Lume-specific validation —
//! that's Strategy B (Phase 5+, requires upstream protocol changes).
//!
//! # NoteData Encoding
//!
//! Each piece of Lume settlement data becomes a `NoteDataEntry` with a
//! well-known key and a jammed Noun value:
//!
//! | Key             | Value (Noun)              | Description                          |
//! |-----------------|---------------------------|--------------------------------------|
//! | `lume-v`        | `@ud` (version number)    | Schema version for forward compat    |
//! | `lume-vid`      | `@ud` (vessel ID)         | Vessel that produced the settlement  |
//! | `lume-root`     | `@` (tip5 digest atom)    | Merkle root of the committed tree    |
//! | `lume-nid`      | `@ud` (note ID)           | Lume's internal note identifier      |
//! | `lume-mhash`    | `@` (tip5 digest atom)    | Hash of the serialized manifest      |
//!
//! # gRPC Client (Phase 3.2)
//!
//! `ChainClient` wraps `PublicNockchainGrpcClient` to provide Lume-specific
//! methods:
//!
//! - **Submit settlement transactions** to a Nockchain node
//! - **Query for Lume Note state** by scanning on-chain notes for Lume NoteData
//! - **Watch for transaction confirmation** via polling with configurable timeout
//! - **Check wallet funding** before attempting settlement

use std::time::Duration;

use anyhow::{Context, Result};
use nockapp::noun::slab::{NockJammer, NounSlab};
use nockchain_types::tx_engine::v1::note::{NoteData, NoteDataEntry};
use nockvm::noun::{IndirectAtom, Noun, D};

use crate::merkle::hash_leaf;
use crate::noun_builder::tip5_to_atom_le_bytes;

use crate::types::*;

// Re-export nockchain types needed for FirstName computation.
use nockchain_types::tx_engine::common::Hash as ChainHash;
use nockchain_types::tx_engine::v1::tx::SpendCondition;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Schema version for Lume NoteData entries. Increment when the encoding changes.
pub const LUME_DATA_VERSION: u64 = 1;

/// NoteData key: schema version.
pub const KEY_VERSION: &str = "lume-v";
/// NoteData key: vessel ID.
pub const KEY_VESSEL_ID: &str = "lume-vid";
/// NoteData key: Merkle root (32-byte SHA-256).
pub const KEY_MERKLE_ROOT: &str = "lume-root";
/// NoteData key: Lume note ID.
pub const KEY_NOTE_ID: &str = "lume-nid";
/// NoteData key: manifest hash (32-byte SHA-256 of the serialized manifest).
pub const KEY_MANIFEST_HASH: &str = "lume-mhash";

// ---------------------------------------------------------------------------
// SettlementData — the Lume-specific data embedded in a Nockchain Note
// ---------------------------------------------------------------------------

/// Lume settlement data that maps into a Nockchain NoteV1's `note_data` field.
///
/// This is the Strategy A payload: everything the chain needs to record that
/// a settlement occurred, without the chain enforcing Lume's verification logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementData {
    /// Schema version (for forward compatibility).
    pub version: u64,
    /// The vessel that produced this settlement.
    pub vessel_id: u64,
    /// tip5 Merkle root of the committed document tree.
    pub merkle_root: Tip5Hash,
    /// Lume's internal note identifier.
    pub note_id: u64,
    /// tip5 hash of the serialized manifest (query + retrievals + prompt + output).
    pub manifest_hash: Tip5Hash,
}

impl SettlementData {
    /// Create a new `SettlementData` from a settled Lume note and its manifest.
    pub fn from_settlement(note: &Note, manifest: &Manifest) -> Self {
        Self {
            version: LUME_DATA_VERSION,
            vessel_id: note.vessel,
            merkle_root: note.root,
            note_id: note.id,
            manifest_hash: manifest_hash(manifest),
        }
    }

    /// Encode this settlement data into Nockchain `NoteData` entries.
    ///
    /// Each field becomes a `NoteDataEntry` with a well-known key and a
    /// jammed Noun value. The jammed format matches what Nockchain's
    /// NoteData encoding expects.
    pub fn to_note_data(&self) -> NoteData {
        let entries = vec![
            jam_u64_entry(KEY_VERSION, self.version),
            jam_u64_entry(KEY_VESSEL_ID, self.vessel_id),
            jam_tip5_entry(KEY_MERKLE_ROOT, &self.merkle_root),
            jam_u64_entry(KEY_NOTE_ID, self.note_id),
            jam_tip5_entry(KEY_MANIFEST_HASH, &self.manifest_hash),
        ];
        NoteData::new(entries)
    }

    /// Decode settlement data from Nockchain `NoteData` entries.
    ///
    /// Looks up each well-known key and cues (deserializes) the jammed Noun
    /// value back into Rust types. Returns an error if required keys are
    /// missing or values can't be decoded.
    pub fn from_note_data(data: &NoteData) -> Result<Self> {
        let version = find_u64_entry(data, KEY_VERSION)
            .context("missing lume-v entry in NoteData")?;

        if version > LUME_DATA_VERSION {
            anyhow::bail!(
                "unsupported Lume NoteData version {version} (max supported: {LUME_DATA_VERSION})"
            );
        }

        let vessel_id = find_u64_entry(data, KEY_VESSEL_ID)
            .context("missing lume-vid entry in NoteData")?;
        let merkle_root = find_hash_entry(data, KEY_MERKLE_ROOT)
            .context("missing lume-root entry in NoteData")?;
        let note_id = find_u64_entry(data, KEY_NOTE_ID)
            .context("missing lume-nid entry in NoteData")?;
        let manifest_hash = find_hash_entry(data, KEY_MANIFEST_HASH)
            .context("missing lume-mhash entry in NoteData")?;

        Ok(Self {
            version,
            vessel_id,
            merkle_root,
            note_id,
            manifest_hash,
        })
    }

    /// Convert the tip5 Merkle root to a Nockchain `Hash`.
    ///
    /// This is used when constructing the `Name` field of a Nockchain Note
    /// or Seed, which requires tip5-based Hash values.
    pub fn merkle_root_as_chain_hash(&self) -> nockchain_types::tx_engine::common::Hash {
        nockchain_types::tx_engine::common::Hash::from_limbs(&self.merkle_root)
    }
}

// ---------------------------------------------------------------------------
// Manifest hashing
// ---------------------------------------------------------------------------

/// Compute tip5 hash of a manifest for on-chain integrity verification.
///
/// Concatenates query + "\n" + each chunk's dat + "\n" + prompt + "\n" + output,
/// then hashes the result as a single atom via `hash_leaf` (tip5 varlen sponge).
pub fn manifest_hash(manifest: &Manifest) -> Tip5Hash {
    let mut content = manifest.query.clone();
    content.push('\n');
    for retrieval in &manifest.results {
        content.push_str(&retrieval.chunk.dat);
        content.push('\n');
    }
    content.push_str(&manifest.prompt);
    content.push('\n');
    content.push_str(&manifest.output);
    hash_leaf(content.as_bytes())
}

// ---------------------------------------------------------------------------
// NoteDataEntry encoding helpers — jam Nouns into entry blobs
// ---------------------------------------------------------------------------

/// Create a NoteDataEntry with a jammed u64 atom value.
fn jam_u64_entry(key: &str, value: u64) -> NoteDataEntry {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let noun = D(value);
    slab.set_root(noun);
    let jammed = slab.jam();
    NoteDataEntry::new(key.to_string(), jammed)
}

/// Create a NoteDataEntry with a jammed tip5 hash atom value.
///
/// Converts the `[u64; 5]` digest to a flat atom via `tip5_to_atom_le_bytes`,
/// matching Hoon's `digest-to-atom:tip5`.
fn jam_tip5_entry(key: &str, hash: &Tip5Hash) -> NoteDataEntry {
    let le_bytes = tip5_to_atom_le_bytes(hash);
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let noun = bytes_to_noun_slab(&mut slab, &le_bytes);
    slab.set_root(noun);
    let jammed = slab.jam();
    NoteDataEntry::new(key.to_string(), jammed)
}

/// Convert a byte slice to a Nock atom in a NounSlab.
fn bytes_to_noun_slab(slab: &mut NounSlab<NockJammer>, bytes: &[u8]) -> Noun {
    if bytes.is_empty() {
        return D(0);
    }
    unsafe {
        let mut indirect = IndirectAtom::new_raw_bytes_ref(slab, bytes);
        indirect.normalize_as_atom().as_noun()
    }
}

// ---------------------------------------------------------------------------
// NoteDataEntry decoding helpers — cue Nouns from entry blobs
// ---------------------------------------------------------------------------

/// Find a NoteDataEntry by key and decode its jammed value as a u64.
fn find_u64_entry(data: &NoteData, key: &str) -> Result<u64> {
    let entry = find_entry(data, key)?;
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    slab.cue_into(entry.blob.clone())
        .context("failed to cue NoteDataEntry blob")?;
    let noun = unsafe { *slab.root() };
    let atom = noun
        .as_atom()
        .map_err(|_| anyhow::anyhow!("expected atom for key '{key}', got cell"))?;
    atom.as_u64()
        .map_err(|_| anyhow::anyhow!("atom for key '{key}' does not fit in u64"))
}

/// Find a NoteDataEntry by key and decode its jammed value as a tip5 hash.
///
/// Reads the atom from NoteData, converts its LE bytes back to a [u64; 5] digest
/// by reversing the base-p polynomial encoding.
fn find_hash_entry(data: &NoteData, key: &str) -> Result<Tip5Hash> {
    let entry = find_entry(data, key)?;
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    slab.cue_into(entry.blob.clone())
        .context("failed to cue NoteDataEntry blob")?;
    let noun = unsafe { *slab.root() };
    let atom = noun
        .as_atom()
        .map_err(|_| anyhow::anyhow!("expected atom for key '{key}', got cell"))?;

    // Extract LE bytes from the atom and reconstruct tip5 limbs.
    let ne_bytes = atom.as_ne_bytes();
    atom_le_bytes_to_tip5(ne_bytes)
        .ok_or_else(|| anyhow::anyhow!("failed to decode tip5 hash for key '{key}'"))
}

/// Reverse of `tip5_to_atom_le_bytes`: reconstruct [u64; 5] from LE atom bytes.
///
/// Performs base-p decomposition: divide the atom value by PRIME repeatedly.
fn atom_le_bytes_to_tip5(bytes: &[u8]) -> Option<Tip5Hash> {
    use nockchain_math::belt::PRIME;

    // Reconstruct the atom value as a vector of u8 (LE) and do base-p decomposition.
    // We need to divide a multi-byte number by PRIME repeatedly.
    let mut data: Vec<u8> = bytes.to_vec();
    let mut limbs = [0u64; 5];

    for limb in &mut limbs {
        // Divide data (LE big integer) by PRIME, get remainder
        let mut remainder: u128 = 0;
        for byte in data.iter_mut().rev() {
            remainder = (remainder << 8) | (*byte as u128);
            *byte = (remainder / (PRIME as u128)) as u8;
            remainder %= PRIME as u128;
        }
        *limb = remainder as u64;
        // Trim trailing zeros
        while data.last() == Some(&0) && data.len() > 1 {
            data.pop();
        }
    }
    Some(limbs)
}

/// Find a NoteDataEntry by its key string.
fn find_entry<'a>(data: &'a NoteData, key: &str) -> Result<&'a NoteDataEntry> {
    data.iter()
        .find(|e| e.key == key)
        .ok_or_else(|| anyhow::anyhow!("NoteData key '{key}' not found"))
}

// ---------------------------------------------------------------------------
// ChainConfig — connection and retry configuration
// ---------------------------------------------------------------------------

/// Configuration for chain interaction.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    /// gRPC endpoint URL (e.g., `http://localhost:9090`).
    pub endpoint: String,
    /// How often to poll `transaction_accepted` in `wait_for_acceptance`.
    pub poll_interval: Duration,
    /// Maximum time to wait for transaction acceptance before giving up.
    pub accept_timeout: Duration,
}

impl ChainConfig {
    /// Create a config pointing at a local Nockchain node with sensible defaults.
    pub fn local(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            poll_interval: Duration::from_secs(5),
            accept_timeout: Duration::from_secs(120),
        }
    }
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self::local("http://localhost:9090")
    }
}

// ---------------------------------------------------------------------------
// ChainClient — gRPC client for Nockchain interaction
// ---------------------------------------------------------------------------

/// Client for submitting Lume settlements to a Nockchain node.
///
/// Wraps `PublicNockchainGrpcClient` with Lume-specific methods for:
/// - Submitting pre-signed settlement transactions
/// - Polling for transaction acceptance (block inclusion)
/// - Querying on-chain notes for Lume settlement data
/// - Checking wallet funding status
///
/// Phase 3.3 adds wallet coordination for signing.
pub struct ChainClient {
    client: nockapp_grpc::services::public_nockchain::PublicNockchainGrpcClient,
    config: ChainConfig,
}

impl ChainClient {
    /// Connect to a Nockchain node's public gRPC endpoint.
    pub async fn connect(config: ChainConfig) -> Result<Self> {
        let client =
            nockapp_grpc::services::public_nockchain::PublicNockchainGrpcClient::connect(
                &config.endpoint,
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to Nockchain gRPC at {}: {e:?}", config.endpoint))?;
        Ok(Self { client, config })
    }

    /// Submit a pre-signed raw transaction to the Nockchain node.
    ///
    /// Returns `Ok(())` on acknowledgment. The transaction is not yet in a
    /// block — call [`wait_for_acceptance`] to confirm inclusion.
    ///
    /// In Phase 3.3, higher-level methods will construct the transaction
    /// from Lume settlement data + wallet signing.
    pub async fn submit_transaction(
        &mut self,
        raw_tx: nockchain_types::tx_engine::v1::RawTx,
    ) -> Result<()> {
        self.client
            .wallet_send_transaction(raw_tx)
            .await
            .map_err(|e| anyhow::anyhow!("failed to submit settlement transaction: {e:?}"))?;
        Ok(())
    }

    /// Check if a previously submitted transaction has been accepted into a block.
    ///
    /// Returns `true` if accepted, `false` if not yet accepted.
    pub async fn check_accepted(&mut self, tx_id_base58: &str) -> Result<bool> {
        use nockapp_grpc::pb::public::v2::transaction_accepted_response;

        let tx_id = nockapp_grpc::pb::common::v1::Base58Hash {
            hash: tx_id_base58.to_string(),
        };
        let resp = self
            .client
            .transaction_accepted(tx_id)
            .await
            .map_err(|e| anyhow::anyhow!("failed to check transaction acceptance: {e:?}"))?;

        match resp.result {
            Some(transaction_accepted_response::Result::Accepted(accepted)) => Ok(accepted),
            _ => Ok(false),
        }
    }

    /// Poll until a transaction is accepted into a block, or timeout.
    ///
    /// Uses `config.poll_interval` and `config.accept_timeout`.
    /// Returns `Ok(true)` if accepted, `Ok(false)` if timed out.
    pub async fn wait_for_acceptance(&mut self, tx_id_base58: &str) -> Result<bool> {
        let deadline = tokio::time::Instant::now() + self.config.accept_timeout;

        loop {
            match self.check_accepted(tx_id_base58).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(e) => {
                    // Log but don't fail — the node may be temporarily busy.
                    eprintln!(
                        "  warn: check_accepted error (will retry): {}",
                        e
                    );
                }
            }

            if tokio::time::Instant::now() + self.config.poll_interval > deadline {
                return Ok(false);
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    /// Submit a transaction and wait for it to be accepted.
    ///
    /// Combines `submit_transaction` + `wait_for_acceptance` into a single
    /// call. Returns `true` if the transaction was accepted before timeout.
    pub async fn submit_and_wait(
        &mut self,
        raw_tx: nockchain_types::tx_engine::v1::RawTx,
        tx_id_base58: &str,
    ) -> Result<bool> {
        self.submit_transaction(raw_tx).await?;
        self.wait_for_acceptance(tx_id_base58).await
    }

    /// Get the balance for a full SchnorrPubkey (base58, 97 bytes decoded).
    ///
    /// Use this when you have the full public key (not a PKH hash).
    /// For PKH addresses, use [`get_balance_by_pubkey_or_pkh`] which
    /// tries `Address` first, then falls back to the private gRPC peek.
    pub async fn get_balance(
        &mut self,
        address: &str,
    ) -> Result<nockapp_grpc::pb::common::v2::Balance> {
        use nockapp_grpc::services::public_nockchain::v2::client::BalanceRequest;
        self.client
            .wallet_get_balance(&BalanceRequest::Address(address.to_string()))
            .await
            .map_err(|e| anyhow::anyhow!("failed to get balance: {e:?}"))
    }

    /// Try to get balance, falling back from Address to FirstName selector.
    ///
    /// The public gRPC `Address` selector requires a full SchnorrPubkey
    /// (97 bytes base58). If the address is a PKH hash (~32 bytes base58),
    /// the Address selector will fail. This method tries Address first,
    /// then falls back to treating the string as an error message for
    /// better diagnostics.
    pub async fn get_balance_flexible(
        &mut self,
        address: &str,
    ) -> Result<nockapp_grpc::pb::common::v2::Balance> {
        use nockapp_grpc::services::public_nockchain::v2::client::BalanceRequest;
        match self
            .client
            .wallet_get_balance(&BalanceRequest::Address(address.to_string()))
            .await
        {
            Ok(bal) => Ok(bal),
            Err(_) => {
                // Address selector failed — likely a PKH, not a full pubkey.
                // Try FirstName selector as a fallback (works for hashes).
                self.client
                    .wallet_get_balance(&BalanceRequest::FirstName(address.to_string()))
                    .await
                    .map_err(|e| anyhow::anyhow!(
                        "failed to get balance for '{}' (tried both Address and FirstName selectors): {e:?}",
                        address
                    ))
            }
        }
    }

    /// Get balance by PKH (pubkey hash) using FirstName computation.
    ///
    /// Computes the note FirstName from the PKH + spend condition structure,
    /// then queries via `BalanceRequest::FirstName`. This avoids needing the
    /// full SchnorrPubkey (132-char base58) — only the PKH (58-char) is needed.
    ///
    /// Tries coinbase FirstName first (mining rewards have a timelock), then
    /// falls back to simple P2PKH FirstName (regular transfers).
    pub async fn get_balance_by_pkh(
        &mut self,
        pkh_b58: &str,
        coinbase_timelock_min: u64,
    ) -> Result<nockapp_grpc::pb::common::v2::Balance> {
        use nockapp_grpc::services::public_nockchain::v2::client::BalanceRequest;

        let coinbase_fn = compute_coinbase_first_name(pkh_b58, coinbase_timelock_min)?;
        let simple_fn = compute_simple_first_name(pkh_b58)?;

        // Try coinbase FirstName first (mining rewards).
        match self
            .client
            .wallet_get_balance(&BalanceRequest::FirstName(coinbase_fn.clone()))
            .await
        {
            Ok(bal) if !bal.notes.is_empty() => Ok(bal),
            _ => {
                // Fall back to simple P2PKH FirstName.
                self.client
                    .wallet_get_balance(&BalanceRequest::FirstName(simple_fn))
                    .await
                    .map_err(|e| anyhow::anyhow!(
                        "failed to get balance by PKH '{}' (tried coinbase and simple FirstName): {e:?}",
                        pkh_b58
                    ))
            }
        }
    }

    /// Scan on-chain notes at an address for Lume settlement data.
    ///
    /// Queries the node for all notes associated with `address`, then
    /// iterates their `NoteData` entries looking for Lume keys (`lume-v`,
    /// `lume-vid`, etc.). Returns decoded `SettlementData` for each note
    /// that contains valid Lume data.
    pub async fn find_settlement_notes(
        &mut self,
        address: &str,
    ) -> Result<Vec<SettlementData>> {
        let balance = self.get_balance_flexible(address).await?;
        extract_settlements_from_balance(&balance)
    }

    /// Scan on-chain notes by PKH for Lume settlement data.
    ///
    /// Like [`find_settlement_notes`] but uses PKH-based FirstName queries
    /// instead of requiring a full SchnorrPubkey address.
    pub async fn find_settlement_notes_by_pkh(
        &mut self,
        pkh_b58: &str,
        coinbase_timelock_min: u64,
    ) -> Result<Vec<SettlementData>> {
        let balance = self.get_balance_by_pkh(pkh_b58, coinbase_timelock_min).await?;
        extract_settlements_from_balance(&balance)
    }

    /// Look up a specific Lume settlement note by its note ID.
    ///
    /// Scans all notes at `address` and returns the first one matching
    /// the given Lume `note_id`.
    pub async fn find_settlement_by_id(
        &mut self,
        address: &str,
        note_id: u64,
    ) -> Result<Option<SettlementData>> {
        let settlements = self.find_settlement_notes(address).await?;
        Ok(settlements.into_iter().find(|s| s.note_id == note_id))
    }

    /// Get the underlying config.
    pub fn config(&self) -> &ChainConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// NoteData extraction from protobuf Balance entries
// ---------------------------------------------------------------------------

/// Extract `NoteData` from a protobuf Note (v0 or v1 variant).
///
/// Only NoteV1 carries `note_data`; legacy v0 notes are skipped.
fn extract_note_data(
    note: &nockapp_grpc::pb::common::v2::Note,
) -> Option<NoteData> {
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

// ---------------------------------------------------------------------------
// FirstName computation — derive note FirstName from a PKH
// ---------------------------------------------------------------------------

/// Compute the FirstName for coinbase (mining reward) notes at a given PKH.
///
/// Coinbase notes have a P2PKH lock + relative timelock. The FirstName is
/// the hash of the lock root, which includes both the PKH and the timelock.
///
/// Uses the same computation as the wallet's own test:
/// `nockchain-wallet/src/tests.rs:signing_keys_support_rust_first_name_reconstruction_in_fakenet`
pub fn compute_coinbase_first_name(pkh_b58: &str, coinbase_relative_min: u64) -> Result<String> {
    let pkh = ChainHash::from_base58(pkh_b58)
        .map_err(|e| anyhow::anyhow!("invalid PKH base58 '{}': {e:?}", pkh_b58))?;
    let sc = SpendCondition::coinbase_pkh(pkh, coinbase_relative_min);
    let first_name = sc
        .first_name()
        .map_err(|e| anyhow::anyhow!("failed to compute coinbase FirstName: {e:?}"))?;
    Ok(first_name.to_base58())
}

/// Compute the FirstName for simple P2PKH (transfer) notes at a given PKH.
///
/// Simple P2PKH notes have only a PKH lock (no timelock). Used for regular
/// transfers and settlement outputs.
pub fn compute_simple_first_name(pkh_b58: &str) -> Result<String> {
    let pkh = ChainHash::from_base58(pkh_b58)
        .map_err(|e| anyhow::anyhow!("invalid PKH base58 '{}': {e:?}", pkh_b58))?;
    let sc = SpendCondition::simple_pkh(pkh);
    let first_name = sc
        .first_name()
        .map_err(|e| anyhow::anyhow!("failed to compute simple FirstName: {e:?}"))?;
    Ok(first_name.to_base58())
}

/// Extract settlement data from a balance response.
fn extract_settlements_from_balance(
    balance: &nockapp_grpc::pb::common::v2::Balance,
) -> Result<Vec<SettlementData>> {
    let mut settlements = Vec::new();
    for entry in &balance.notes {
        let note_data = match &entry.note {
            Some(note) => extract_note_data(note),
            None => continue,
        };
        if let Some(data) = note_data {
            if let Ok(sd) = SettlementData::from_note_data(&data) {
                settlements.push(sd);
            }
        }
    }
    Ok(settlements)
}

// ---------------------------------------------------------------------------
// Display / Debug helpers
// ---------------------------------------------------------------------------

impl std::fmt::Display for SettlementData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Settlement(v={}, vessel={}, note={}, root={}, manifest={})",
            self.version,
            self.vessel_id,
            self.note_id,
            crate::merkle::format_tip5(&self.merkle_root),
            crate::merkle::format_tip5(&self.manifest_hash),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_note() -> Note {
        Note {
            id: 42,
            vessel: 7,
            root: [0xAA; 5],
            state: NoteState::Pending,
        }
    }

    fn test_manifest() -> Manifest {
        Manifest {
            query: "What is the revenue?".to_string(),
            results: vec![Retrieval {
                chunk: Chunk {
                    id: 0,
                    dat: "Q3 revenue: $4.2M".to_string(),
                },
                proof: vec![],
                score: 950_000,
            }],
            prompt: "What is the revenue?\nQ3 revenue: $4.2M".to_string(),
            output: "Revenue is $4.2M".to_string(),
        }
    }

    #[test]
    fn settlement_data_roundtrip() {
        let note = test_note();
        let manifest = test_manifest();

        let data = SettlementData::from_settlement(&note, &manifest);
        assert_eq!(data.version, LUME_DATA_VERSION);
        assert_eq!(data.vessel_id, 7);
        assert_eq!(data.note_id, 42);
        assert_eq!(data.merkle_root, [0xAA; 5]);

        // Encode to NoteData
        let note_data = data.to_note_data();
        assert_eq!(note_data.iter().count(), 5);

        // Decode back
        let decoded = SettlementData::from_note_data(&note_data)
            .expect("decode should succeed");

        assert_eq!(decoded.version, data.version);
        assert_eq!(decoded.vessel_id, data.vessel_id);
        assert_eq!(decoded.note_id, data.note_id);
        assert_eq!(decoded.merkle_root, data.merkle_root);
        assert_eq!(decoded.manifest_hash, data.manifest_hash);
    }

    #[test]
    fn manifest_hash_deterministic() {
        let m = test_manifest();
        let h1 = manifest_hash(&m);
        let h2 = manifest_hash(&m);
        assert_eq!(h1, h2, "manifest hash must be deterministic");
        assert_ne!(h1, [0u64; 5], "hash must not be zero");
    }

    #[test]
    fn manifest_hash_changes_with_content() {
        let m1 = test_manifest();
        let mut m2 = test_manifest();
        m2.output = "Different output".to_string();

        let h1 = manifest_hash(&m1);
        let h2 = manifest_hash(&m2);
        assert_ne!(h1, h2, "different manifests must produce different hashes");
    }

    #[test]
    fn note_data_keys_present() {
        let data = SettlementData {
            version: 1,
            vessel_id: 7,
            merkle_root: [0xBB; 5],
            note_id: 99,
            manifest_hash: [0xCC; 5],
        };
        let note_data = data.to_note_data();

        let keys: Vec<&str> = note_data.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&KEY_VERSION));
        assert!(keys.contains(&KEY_VESSEL_ID));
        assert!(keys.contains(&KEY_MERKLE_ROOT));
        assert!(keys.contains(&KEY_NOTE_ID));
        assert!(keys.contains(&KEY_MANIFEST_HASH));
    }

    #[test]
    fn decode_rejects_missing_keys() {
        let empty = NoteData::new(vec![]);
        let result = SettlementData::from_note_data(&empty);
        assert!(result.is_err());
    }

    #[test]
    fn decode_rejects_future_version() {
        let data = SettlementData {
            version: 999,
            vessel_id: 1,
            merkle_root: [0; 5],
            note_id: 1,
            manifest_hash: [0; 5],
        };
        let note_data = data.to_note_data();
        let result = SettlementData::from_note_data(&note_data);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("unsupported"),
            "error should mention unsupported version"
        );
    }

    #[test]
    fn merkle_root_chain_hash_conversion() {
        let data = SettlementData {
            version: 1,
            vessel_id: 7,
            merkle_root: [1, 2, 3, 4, 5],
            note_id: 1,
            manifest_hash: [0; 5],
        };
        let hash = data.merkle_root_as_chain_hash();
        // Hash::from_limbs preserves limb values directly
        assert_eq!(hash.to_array(), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn display_format() {
        let data = SettlementData {
            version: 1,
            vessel_id: 7,
            merkle_root: [0xAA; 5],
            note_id: 42,
            manifest_hash: [0xBB; 5],
        };
        let s = format!("{data}");
        assert!(s.contains("vessel=7"));
        assert!(s.contains("note=42"));
    }

    // --- Phase 3.2 tests ---

    #[test]
    fn chain_config_defaults() {
        let cfg = ChainConfig::default();
        assert_eq!(cfg.endpoint, "http://localhost:9090");
        assert_eq!(cfg.poll_interval, Duration::from_secs(5));
        assert_eq!(cfg.accept_timeout, Duration::from_secs(120));
    }

    #[test]
    fn chain_config_local() {
        let cfg = ChainConfig::local("http://node:8080");
        assert_eq!(cfg.endpoint, "http://node:8080");
        assert_eq!(cfg.poll_interval, Duration::from_secs(5));
        assert_eq!(cfg.accept_timeout, Duration::from_secs(120));
    }

    #[test]
    fn settlement_data_from_settlement_computes_manifest_hash() {
        let note = test_note();
        let manifest = test_manifest();
        let sd = SettlementData::from_settlement(&note, &manifest);

        // Manifest hash must match the standalone function
        assert_eq!(sd.manifest_hash, manifest_hash(&manifest));
        assert_eq!(sd.version, LUME_DATA_VERSION);
    }

    #[test]
    fn settlement_data_roundtrip_preserves_all_fields() {
        // Use non-trivial values to ensure encoding isn't swallowing data.
        // vessel_id must fit in Nock direct atom (63-bit max).
        let data = SettlementData {
            version: 1,
            vessel_id: (1u64 << 63) - 1,
            merkle_root: [1, 2, 3, 4, 5],
            note_id: 123_456_789,
            manifest_hash: [100, 200, 300, 400, 500],
        };
        let note_data = data.to_note_data();
        let decoded = SettlementData::from_note_data(&note_data).unwrap();
        assert_eq!(decoded, data);
    }

    // --- FirstName computation tests (ISSUE-004 fix) ---

    /// The MINING_PKH from .env.fakenet — a real v1 PKH used in our fakenet.
    const TEST_MINING_PKH: &str = "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV";

    #[test]
    fn coinbase_first_name_computes_from_pkh() {
        let fn_str = compute_coinbase_first_name(TEST_MINING_PKH, 1)
            .expect("coinbase first_name should compute from valid PKH");
        assert!(!fn_str.is_empty(), "first_name base58 must not be empty");
    }

    #[test]
    fn simple_first_name_computes_from_pkh() {
        let fn_str = compute_simple_first_name(TEST_MINING_PKH)
            .expect("simple first_name should compute from valid PKH");
        assert!(!fn_str.is_empty(), "first_name base58 must not be empty");
    }

    #[test]
    fn coinbase_and_simple_first_names_differ() {
        let coinbase = compute_coinbase_first_name(TEST_MINING_PKH, 1).unwrap();
        let simple = compute_simple_first_name(TEST_MINING_PKH).unwrap();
        assert_ne!(
            coinbase, simple,
            "coinbase and simple first_names must differ (timelock changes the lock root)"
        );
    }

    #[test]
    fn first_name_computation_is_deterministic() {
        let fn1 = compute_coinbase_first_name(TEST_MINING_PKH, 1).unwrap();
        let fn2 = compute_coinbase_first_name(TEST_MINING_PKH, 1).unwrap();
        assert_eq!(fn1, fn2, "same PKH + timelock must produce identical first_name");
    }

    #[test]
    fn first_name_rejects_invalid_pkh() {
        let result = compute_coinbase_first_name("not-a-valid-base58-hash", 1);
        assert!(result.is_err(), "invalid PKH should produce an error");
    }

    #[test]
    fn different_timelock_produces_different_first_name() {
        let fn1 = compute_coinbase_first_name(TEST_MINING_PKH, 1).unwrap();
        let fn2 = compute_coinbase_first_name(TEST_MINING_PKH, 10).unwrap();
        assert_ne!(
            fn1, fn2,
            "different coinbase timelock values must produce different first_names"
        );
    }
}
