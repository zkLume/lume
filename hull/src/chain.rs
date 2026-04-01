//! Chain integration — maps Vesl settlement data to Nockchain's transaction model.
//!
//! Phases 3.1 + 3.2 of the DEV.md roadmap. Implements **Strategy A: Data-in-Note**.
//!
//! # Strategy A
//!
//! Vesl settlement data (hull ID, Merkle root, manifest hash) is embedded
//! in the `note_data` field of a standard Nockchain `NoteV1` / `Seed`.
//! The on-chain guarantee is: "a Note exists with this data, signed by
//! Vesl's key." The chain itself does not enforce Vesl-specific validation —
//! that's Strategy B (Phase 5+, requires upstream protocol changes).
//!
//! # NoteData Encoding
//!
//! Each piece of Vesl settlement data becomes a `NoteDataEntry` with a
//! well-known key and a jammed Noun value:
//!
//! | Key             | Value (Noun)              | Description                          |
//! |-----------------|---------------------------|--------------------------------------|
//! | `vesl-v`        | `@ud` (version number)    | Schema version for forward compat    |
//! | `vesl-vid`      | `@ud` (hull ID)         | Hull that produced the settlement  |
//! | `vesl-root`     | `@` (tip5 digest atom)    | Merkle root of the committed tree    |
//! | `vesl-nid`      | `@ud` (note ID)           | Vesl's internal note identifier      |
//! | `vesl-mhash`    | `@` (tip5 digest atom)    | Hash of the serialized manifest      |
//!
//! # gRPC Client (Phase 3.2)
//!
//! `ChainClient` wraps `PublicNockchainGrpcClient` to provide Vesl-specific
//! methods:
//!
//! - **Submit settlement transactions** to a Nockchain node
//! - **Query for Vesl Note state** by scanning on-chain notes for Vesl NoteData
//! - **Watch for transaction confirmation** via polling with configurable timeout
//! - **Check wallet funding** before attempting settlement

use std::time::Duration;

use anyhow::{Context, Result};
use nockapp::noun::slab::{NockJammer, NounSlab};
use nockchain_types::tx_engine::v1::note::{NoteData, NoteDataEntry};
use nockvm::noun::{IndirectAtom, Noun, D, T};

use crate::merkle::hash_leaf;

use crate::types::*;

// Re-export nockchain types needed for FirstName computation.
use nockchain_types::tx_engine::common::Hash as ChainHash;
use nockchain_types::tx_engine::v1::tx::SpendCondition;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Schema version for Vesl NoteData entries. Increment when the encoding changes.
pub const VESL_DATA_VERSION: u64 = 1;

/// NoteData key: schema version.
pub const KEY_VERSION: &str = "vesl-v";
/// NoteData key: hull ID.
pub const KEY_HULL_ID: &str = "vesl-vid";
/// NoteData key: Merkle root (32-byte SHA-256).
pub const KEY_MERKLE_ROOT: &str = "vesl-rt";
/// NoteData key: Vesl note ID.
pub const KEY_NOTE_ID: &str = "vesl-nid";
/// NoteData key: manifest hash (32-byte SHA-256 of the serialized manifest).
pub const KEY_MANIFEST_HASH: &str = "vesl-mh";

// ---------------------------------------------------------------------------
// SettlementData — the Vesl-specific data embedded in a Nockchain Note
// ---------------------------------------------------------------------------

/// Vesl settlement data that maps into a Nockchain NoteV1's `note_data` field.
///
/// This is the Strategy A payload: everything the chain needs to record that
/// a settlement occurred, without the chain enforcing Vesl's verification logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementData {
    /// Schema version (for forward compatibility).
    pub version: u64,
    /// The hull that produced this settlement.
    pub hull_id: u64,
    /// tip5 Merkle root of the committed document tree.
    pub merkle_root: Tip5Hash,
    /// Vesl's internal note identifier.
    pub note_id: u64,
    /// tip5 hash of the serialized manifest (query + retrievals + prompt + output).
    pub manifest_hash: Tip5Hash,
}

impl SettlementData {
    /// Create a new `SettlementData` from a settled Vesl note and its manifest.
    pub fn from_settlement(note: &Note, manifest: &Manifest) -> Self {
        Self {
            version: VESL_DATA_VERSION,
            hull_id: note.hull,
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
            jam_u64_entry(KEY_HULL_ID, self.hull_id),
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
            .context("missing vesl-v entry in NoteData")?;

        if version > VESL_DATA_VERSION {
            anyhow::bail!(
                "unsupported Vesl NoteData version {version} (max supported: {VESL_DATA_VERSION})"
            );
        }

        let hull_id = find_u64_entry(data, KEY_HULL_ID)
            .context("missing vesl-vid entry in NoteData")?;
        let merkle_root = find_hash_entry(data, KEY_MERKLE_ROOT)
            .context("missing vesl-root entry in NoteData")?;
        let note_id = find_u64_entry(data, KEY_NOTE_ID)
            .context("missing vesl-nid entry in NoteData")?;
        let manifest_hash = find_hash_entry(data, KEY_MANIFEST_HASH)
            .context("missing vesl-mhash entry in NoteData")?;

        Ok(Self {
            version,
            hull_id,
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

/// Create a NoteDataEntry with a jammed tip5 hash value.
///
/// Encodes the `[u64; 5]` digest as a null-terminated list of 5 u64 atoms:
/// `[limb0 limb1 limb2 limb3 limb4 0]`. Each limb is a Belt-sized value
/// (< 2^64) which is required by the z-map tree hasher's `leaf_sequence`.
/// Limbs > DIRECT_MAX (2^63 - 1) are stored as indirect atoms.
fn jam_tip5_entry(key: &str, hash: &Tip5Hash) -> NoteDataEntry {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    // Build a Nock list: [h[0] [h[1] [h[2] [h[3] [h[4] 0]]]]]
    let mut noun = D(0); // null terminator
    for &limb in hash.iter().rev() {
        let limb_noun = u64_to_noun(&mut slab, limb);
        noun = T(&mut slab, &[limb_noun, noun]);
    }
    slab.set_root(noun);
    let jammed = slab.jam();
    NoteDataEntry::new(key.to_string(), jammed)
}

/// Convert a u64 to a Nock noun, using IndirectAtom for values > DIRECT_MAX.
fn u64_to_noun(slab: &mut NounSlab<NockJammer>, val: u64) -> Noun {
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
/// Reads a 5-element Nock list `[limb0 limb1 limb2 limb3 limb4 0]` from the
/// NoteData entry and reconstructs the `[u64; 5]` digest.
fn find_hash_entry(data: &NoteData, key: &str) -> Result<Tip5Hash> {
    let entry = find_entry(data, key)?;
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    slab.cue_into(entry.blob.clone())
        .context("failed to cue NoteDataEntry blob")?;
    let mut noun = unsafe { *slab.root() };
    let mut limbs = [0u64; 5];
    for (i, limb) in limbs.iter_mut().enumerate() {
        let cell = noun
            .as_cell()
            .map_err(|_| anyhow::anyhow!("tip5 hash list too short at index {i} for key '{key}'"))?;
        let atom = cell
            .head()
            .as_atom()
            .map_err(|_| anyhow::anyhow!("tip5 limb {i} is not an atom for key '{key}'"))?;
        *limb = atom
            .as_u64()
            .map_err(|_| anyhow::anyhow!("tip5 limb {i} exceeds u64 for key '{key}'"))?;
        noun = cell.tail();
    }
    Ok(limbs)
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

/// Client for submitting Vesl settlements to a Nockchain node.
///
/// Wraps `PublicNockchainGrpcClient` with Vesl-specific methods for:
/// - Submitting pre-signed settlement transactions
/// - Polling for transaction acceptance (block inclusion)
/// - Querying on-chain notes for Vesl settlement data
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
    /// from Vesl settlement data + wallet signing.
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

    /// Scan on-chain notes at an address for Vesl settlement data.
    ///
    /// Queries the node for all notes associated with `address`, then
    /// iterates their `NoteData` entries looking for Vesl keys (`vesl-v`,
    /// `vesl-vid`, etc.). Returns decoded `SettlementData` for each note
    /// that contains valid Vesl data.
    pub async fn find_settlement_notes(
        &mut self,
        address: &str,
    ) -> Result<Vec<SettlementData>> {
        let balance = self.get_balance_flexible(address).await?;
        extract_settlements_from_balance(&balance)
    }

    /// Scan on-chain notes by PKH for Vesl settlement data.
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

    /// Look up a specific Vesl settlement note by its note ID.
    ///
    /// Scans all notes at `address` and returns the first one matching
    /// the given Vesl `note_id`.
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
pub fn extract_settlements_from_balance(
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
// SpendableUtxo — extracted UTXO info from protobuf Balance (Phase 3.5.3)
// ---------------------------------------------------------------------------

/// A spendable UTXO extracted from a protobuf Balance response.
///
/// Contains the fields needed to construct a `SettlementTxParams` for
/// `build_settlement_tx`. Protobuf `BalanceEntry` notes are converted
/// to this struct for use in the Rust transaction builder.
#[derive(Debug, Clone)]
pub struct SpendableUtxo {
    /// The note's Name (first/last hash pair) — used as `input_name`.
    pub name: ChainHash,
    /// The note's last-name hash.
    pub last_name: ChainHash,
    /// The note's amount in nicks.
    pub amount: u64,
    /// Whether this is a NoteV1 (supports NoteData).
    pub is_v1: bool,
    /// Vesl settlement data if present in the note's NoteData.
    pub settlement: Option<SettlementData>,
}

impl SpendableUtxo {
    /// The note's first-name hash.
    pub fn first_name(&self) -> &ChainHash {
        &self.name
    }

    /// The note's last-name hash.
    pub fn last_name(&self) -> &ChainHash {
        &self.last_name
    }
}

/// Extract spendable UTXO info from a protobuf Balance response.
///
/// Parses each `BalanceEntry` to extract the note name, amount, and
/// any embedded Vesl settlement data. Skips entries with missing data.
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

        // Parse name hashes from protobuf
        let first_name = match &pb_name.first {
            Some(h) => chain_hash_from_pb(h),
            None => continue,
        };
        let last_name = match &pb_name.last {
            Some(h) => chain_hash_from_pb(h),
            None => continue,
        };

        // Parse note version and amount
        use nockapp_grpc::pb::common::v2::note::NoteVersion;
        let (is_v1, amount, settlement) = match &note.note_version {
            Some(NoteVersion::V1(v1)) => {
                let amt = v1.assets.as_ref().map_or(0, |n| n.value);
                let settlement = v1.note_data.as_ref().and_then(|pd| {
                    let entries: Vec<nockchain_types::tx_engine::v1::note::NoteDataEntry> = pd
                        .entries
                        .iter()
                        .map(|e| {
                            nockchain_types::tx_engine::v1::note::NoteDataEntry::new(
                                e.key.clone(),
                                e.blob.clone().into(),
                            )
                        })
                        .collect();
                    if entries.is_empty() {
                        return None;
                    }
                    let nd = NoteData::new(entries);
                    SettlementData::from_note_data(&nd).ok()
                });
                (true, amt, settlement)
            }
            Some(NoteVersion::Legacy(v0)) => {
                let amt = v0.assets.as_ref().map_or(0, |n| n.value);
                (false, amt, None)
            }
            None => continue,
        };

        utxos.push(SpendableUtxo {
            name: first_name,
            last_name,
            amount,
            is_v1,
            settlement,
        });
    }
    utxos
}

/// Convert a protobuf Hash to a nockchain-types Hash.
fn chain_hash_from_pb(pb: &nockapp_grpc::pb::common::v1::Hash) -> ChainHash {
    ChainHash::from_limbs(&[
        pb.belt_1.as_ref().map_or(0, |b| b.value),
        pb.belt_2.as_ref().map_or(0, |b| b.value),
        pb.belt_3.as_ref().map_or(0, |b| b.value),
        pb.belt_4.as_ref().map_or(0, |b| b.value),
        pb.belt_5.as_ref().map_or(0, |b| b.value),
    ])
}

// ---------------------------------------------------------------------------
// On-Chain Settlement Confirmation (Phase 3.5.3)
// ---------------------------------------------------------------------------

/// Result of an on-chain settlement confirmation.
#[derive(Debug, Clone)]
pub struct SettlementConfirmation {
    /// The decoded settlement data from the on-chain note.
    pub on_chain: SettlementData,
    /// Whether all fields match the expected settlement.
    pub verified: bool,
    /// Human-readable description of any mismatches.
    pub mismatches: Vec<String>,
}

impl SettlementData {
    /// Compare this settlement data against an expected value, reporting mismatches.
    ///
    /// Returns a list of human-readable mismatch descriptions. An empty list
    /// means all fields match.
    pub fn diff(&self, expected: &SettlementData) -> Vec<String> {
        let mut mismatches = Vec::new();
        if self.version != expected.version {
            mismatches.push(format!(
                "version: on-chain={}, expected={}",
                self.version, expected.version
            ));
        }
        if self.hull_id != expected.hull_id {
            mismatches.push(format!(
                "hull_id: on-chain={}, expected={}",
                self.hull_id, expected.hull_id
            ));
        }
        if self.note_id != expected.note_id {
            mismatches.push(format!(
                "note_id: on-chain={}, expected={}",
                self.note_id, expected.note_id
            ));
        }
        if self.merkle_root != expected.merkle_root {
            mismatches.push(format!(
                "merkle_root: on-chain={}, expected={}",
                crate::merkle::format_tip5(&self.merkle_root),
                crate::merkle::format_tip5(&expected.merkle_root),
            ));
        }
        if self.manifest_hash != expected.manifest_hash {
            mismatches.push(format!(
                "manifest_hash: on-chain={}, expected={}",
                crate::merkle::format_tip5(&self.manifest_hash),
                crate::merkle::format_tip5(&expected.manifest_hash),
            ));
        }
        mismatches
    }
}

impl ChainClient {
    /// Confirm a specific settlement landed on-chain with matching data.
    ///
    /// Queries the chain for notes at the given PKH, finds one whose
    /// `note_id` matches the expected settlement, and verifies all fields.
    /// Returns `SettlementConfirmation` with match details.
    ///
    /// Returns an error if the chain query fails. Returns `Ok(None)` if
    /// no note with matching `note_id` is found.
    pub async fn confirm_settlement(
        &mut self,
        pkh_b58: &str,
        coinbase_timelock_min: u64,
        expected: &SettlementData,
    ) -> Result<Option<SettlementConfirmation>> {
        let settlements = self
            .find_settlement_notes_by_pkh(pkh_b58, coinbase_timelock_min)
            .await?;

        let on_chain = match settlements.into_iter().find(|s| s.note_id == expected.note_id) {
            Some(s) => s,
            None => return Ok(None),
        };

        let mismatches = on_chain.diff(expected);
        let verified = mismatches.is_empty();

        Ok(Some(SettlementConfirmation {
            on_chain,
            verified,
            mismatches,
        }))
    }

    /// Confirm a settlement by scanning all notes, not just by PKH.
    ///
    /// Uses `find_settlement_notes` with the flexible address selector.
    /// Useful when the exact FirstName computation is uncertain.
    pub async fn confirm_settlement_by_address(
        &mut self,
        address: &str,
        expected: &SettlementData,
    ) -> Result<Option<SettlementConfirmation>> {
        let settlements = self.find_settlement_notes(address).await?;

        let on_chain = match settlements.into_iter().find(|s| s.note_id == expected.note_id) {
            Some(s) => s,
            None => return Ok(None),
        };

        let mismatches = on_chain.diff(expected);
        let verified = mismatches.is_empty();

        Ok(Some(SettlementConfirmation {
            on_chain,
            verified,
            mismatches,
        }))
    }
}

// ---------------------------------------------------------------------------
// Display / Debug helpers
// ---------------------------------------------------------------------------

impl std::fmt::Display for SettlementData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Settlement(v={}, hull={}, note={}, root={}, manifest={})",
            self.version,
            self.hull_id,
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
            hull: 7,
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
        assert_eq!(data.version, VESL_DATA_VERSION);
        assert_eq!(data.hull_id, 7);
        assert_eq!(data.note_id, 42);
        assert_eq!(data.merkle_root, [0xAA; 5]);

        // Encode to NoteData
        let note_data = data.to_note_data();
        assert_eq!(note_data.iter().count(), 5);

        // Decode back
        let decoded = SettlementData::from_note_data(&note_data)
            .expect("decode should succeed");

        assert_eq!(decoded.version, data.version);
        assert_eq!(decoded.hull_id, data.hull_id);
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
            hull_id: 7,
            merkle_root: [0xBB; 5],
            note_id: 99,
            manifest_hash: [0xCC; 5],
        };
        let note_data = data.to_note_data();

        let keys: Vec<&str> = note_data.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&KEY_VERSION));
        assert!(keys.contains(&KEY_HULL_ID));
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
            hull_id: 1,
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
            hull_id: 7,
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
            hull_id: 7,
            merkle_root: [0xAA; 5],
            note_id: 42,
            manifest_hash: [0xBB; 5],
        };
        let s = format!("{data}");
        assert!(s.contains("hull=7"));
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
        assert_eq!(sd.version, VESL_DATA_VERSION);
    }

    #[test]
    fn settlement_data_roundtrip_preserves_all_fields() {
        // Use non-trivial values to ensure encoding isn't swallowing data.
        // hull_id must fit in Nock direct atom (63-bit max).
        let data = SettlementData {
            version: 1,
            hull_id: (1u64 << 63) - 1,
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

    // --- Phase 3.5.3 tests: On-Chain Confirmation ---

    #[test]
    fn settlement_diff_reports_matching() {
        let data = SettlementData {
            version: 1,
            hull_id: 7,
            merkle_root: [1, 2, 3, 4, 5],
            note_id: 42,
            manifest_hash: [6, 7, 8, 9, 10],
        };
        let mismatches = data.diff(&data);
        assert!(mismatches.is_empty(), "identical data should have no mismatches");
    }

    #[test]
    fn settlement_diff_reports_all_mismatches() {
        let expected = SettlementData {
            version: 1,
            hull_id: 7,
            merkle_root: [1, 2, 3, 4, 5],
            note_id: 42,
            manifest_hash: [6, 7, 8, 9, 10],
        };
        let on_chain = SettlementData {
            version: 1,
            hull_id: 99,
            merkle_root: [10, 20, 30, 40, 50],
            note_id: 42,
            manifest_hash: [60, 70, 80, 90, 100],
        };
        let mismatches = on_chain.diff(&expected);
        assert_eq!(mismatches.len(), 3, "should report hull_id, merkle_root, manifest_hash");
        assert!(mismatches.iter().any(|m| m.contains("hull_id")));
        assert!(mismatches.iter().any(|m| m.contains("merkle_root")));
        assert!(mismatches.iter().any(|m| m.contains("manifest_hash")));
    }

    #[test]
    fn settlement_diff_reports_version_mismatch() {
        let a = SettlementData {
            version: 1,
            hull_id: 7,
            merkle_root: [0; 5],
            note_id: 1,
            manifest_hash: [0; 5],
        };
        let b = SettlementData {
            version: 2,
            ..a.clone()
        };
        let mismatches = a.diff(&b);
        assert_eq!(mismatches.len(), 1);
        assert!(mismatches[0].contains("version"));
    }

    #[test]
    fn extract_settlements_from_balance_handles_empty() {
        let balance = nockapp_grpc::pb::common::v2::Balance {
            notes: vec![],
            height: None,
            block_id: None,
            page: None,
        };
        let settlements = extract_settlements_from_balance(&balance).unwrap();
        assert!(settlements.is_empty());
    }

    #[test]
    fn extract_spendable_utxos_handles_empty() {
        let balance = nockapp_grpc::pb::common::v2::Balance {
            notes: vec![],
            height: None,
            block_id: None,
            page: None,
        };
        let utxos = extract_spendable_utxos(&balance);
        assert!(utxos.is_empty());
    }

    #[test]
    fn settlement_encode_decode_via_synthetic_proto_balance() {
        // Build settlement data
        let data = SettlementData {
            version: 1,
            hull_id: 7,
            merkle_root: [100, 200, 300, 400, 500],
            note_id: 42,
            manifest_hash: [10, 20, 30, 40, 50],
        };

        // Encode to NoteData (JAM'd entries)
        let note_data = data.to_note_data();

        // Convert NoteData entries to protobuf NoteDataEntry format
        let pb_entries: Vec<nockapp_grpc::pb::common::v2::NoteDataEntry> = note_data
            .iter()
            .map(|e| nockapp_grpc::pb::common::v2::NoteDataEntry {
                key: e.key.clone(),
                blob: e.blob.to_vec(),
            })
            .collect();

        // Build a synthetic protobuf Balance with a V1 note containing our entries
        let pb_note = nockapp_grpc::pb::common::v2::Note {
            note_version: Some(
                nockapp_grpc::pb::common::v2::note::NoteVersion::V1(
                    nockapp_grpc::pb::common::v2::NoteV1 {
                        version: Some(nockapp_grpc::pb::common::v1::NoteVersion { value: 1 }),
                        origin_page: Some(nockapp_grpc::pb::common::v1::BlockHeight { value: 5 }),
                        name: Some(nockapp_grpc::pb::common::v1::Name {
                            first: Some(nockapp_grpc::pb::common::v1::Hash {
                                belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 1 }),
                                belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 2 }),
                                belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 3 }),
                                belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 4 }),
                                belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 5 }),
                            }),
                            last: Some(nockapp_grpc::pb::common::v1::Hash {
                                belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 10 }),
                                belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 20 }),
                                belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 30 }),
                                belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 40 }),
                                belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 50 }),
                            }),
                        }),
                        note_data: Some(nockapp_grpc::pb::common::v2::NoteData {
                            entries: pb_entries,
                        }),
                        assets: Some(nockapp_grpc::pb::common::v1::Nicks { value: 97_000 }),
                    },
                ),
            ),
        };

        let pb_balance_entry = nockapp_grpc::pb::common::v2::BalanceEntry {
            name: Some(nockapp_grpc::pb::common::v1::Name {
                first: Some(nockapp_grpc::pb::common::v1::Hash {
                    belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 1 }),
                    belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 2 }),
                    belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 3 }),
                    belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 4 }),
                    belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 5 }),
                }),
                last: Some(nockapp_grpc::pb::common::v1::Hash {
                    belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 10 }),
                    belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 20 }),
                    belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 30 }),
                    belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 40 }),
                    belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 50 }),
                }),
            }),
            note: Some(pb_note),
        };

        let pb_balance = nockapp_grpc::pb::common::v2::Balance {
            notes: vec![pb_balance_entry],
            height: Some(nockapp_grpc::pb::common::v1::BlockHeight { value: 10 }),
            block_id: None,
            page: None,
        };

        // --- Test extract_settlements_from_balance ---
        let settlements = extract_settlements_from_balance(&pb_balance).unwrap();
        assert_eq!(settlements.len(), 1, "should find exactly 1 Vesl settlement");
        assert_eq!(settlements[0], data, "decoded settlement must match original");

        // --- Test extract_spendable_utxos ---
        let utxos = extract_spendable_utxos(&pb_balance);
        assert_eq!(utxos.len(), 1, "should find exactly 1 UTXO");
        assert!(utxos[0].is_v1, "must be v1 note");
        assert_eq!(utxos[0].amount, 97_000, "amount must match");
        assert!(utxos[0].settlement.is_some(), "must decode Vesl settlement");
        assert_eq!(utxos[0].settlement.as_ref().unwrap(), &data);

        // --- Test SettlementData::diff ---
        let mismatches = settlements[0].diff(&data);
        assert!(mismatches.is_empty(), "matching data should have no diffs");
    }

    #[test]
    fn extract_spendable_utxos_skips_non_vesl_notes() {
        // A V1 note with non-Vesl NoteData entries
        let pb_note = nockapp_grpc::pb::common::v2::Note {
            note_version: Some(
                nockapp_grpc::pb::common::v2::note::NoteVersion::V1(
                    nockapp_grpc::pb::common::v2::NoteV1 {
                        version: Some(nockapp_grpc::pb::common::v1::NoteVersion { value: 1 }),
                        origin_page: Some(nockapp_grpc::pb::common::v1::BlockHeight { value: 1 }),
                        name: Some(nockapp_grpc::pb::common::v1::Name {
                            first: Some(nockapp_grpc::pb::common::v1::Hash {
                                belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 1 }),
                                belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                                belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                                belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                                belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                            }),
                            last: Some(nockapp_grpc::pb::common::v1::Hash {
                                belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 2 }),
                                belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                                belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                                belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                                belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                            }),
                        }),
                        note_data: Some(nockapp_grpc::pb::common::v2::NoteData {
                            entries: vec![nockapp_grpc::pb::common::v2::NoteDataEntry {
                                key: "lock".to_string(),
                                blob: vec![1, 2, 3],
                            }],
                        }),
                        assets: Some(nockapp_grpc::pb::common::v1::Nicks { value: 50_000 }),
                    },
                ),
            ),
        };

        let pb_balance = nockapp_grpc::pb::common::v2::Balance {
            notes: vec![nockapp_grpc::pb::common::v2::BalanceEntry {
                name: Some(nockapp_grpc::pb::common::v1::Name {
                    first: Some(nockapp_grpc::pb::common::v1::Hash {
                        belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 1 }),
                        belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                        belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                        belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                        belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                    }),
                    last: Some(nockapp_grpc::pb::common::v1::Hash {
                        belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 2 }),
                        belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                        belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                        belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                        belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 0 }),
                    }),
                }),
                note: Some(pb_note),
            }],
            height: None,
            block_id: None,
            page: None,
        };

        // extract_settlements should find 0 (non-Vesl note)
        let settlements = extract_settlements_from_balance(&pb_balance).unwrap();
        assert!(settlements.is_empty(), "non-Vesl notes should be skipped");

        // extract_spendable_utxos should find 1 UTXO with no settlement
        let utxos = extract_spendable_utxos(&pb_balance);
        assert_eq!(utxos.len(), 1);
        assert!(utxos[0].settlement.is_none(), "non-Vesl note should have no settlement");
        assert_eq!(utxos[0].amount, 50_000);
    }

    #[test]
    fn chain_hash_from_pb_converts_correctly() {
        let pb_hash = nockapp_grpc::pb::common::v1::Hash {
            belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 100 }),
            belt_2: Some(nockapp_grpc::pb::common::v1::Belt { value: 200 }),
            belt_3: Some(nockapp_grpc::pb::common::v1::Belt { value: 300 }),
            belt_4: Some(nockapp_grpc::pb::common::v1::Belt { value: 400 }),
            belt_5: Some(nockapp_grpc::pb::common::v1::Belt { value: 500 }),
        };
        let hash = chain_hash_from_pb(&pb_hash);
        assert_eq!(hash.to_array(), [100, 200, 300, 400, 500]);
    }

    #[test]
    fn chain_hash_from_pb_handles_missing_belts() {
        let pb_hash = nockapp_grpc::pb::common::v1::Hash {
            belt_1: Some(nockapp_grpc::pb::common::v1::Belt { value: 42 }),
            belt_2: None,
            belt_3: None,
            belt_4: None,
            belt_5: None,
        };
        let hash = chain_hash_from_pb(&pb_hash);
        assert_eq!(hash.to_array(), [42, 0, 0, 0, 0]);
    }
}
