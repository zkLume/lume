//! Wallet coordination — Phase 3.3 of the DEV.md roadmap.
//!
//! Communicates with a separately funded nockchain-wallet instance
//! via its private gRPC endpoint (default: `localhost:5555`).
//!
//! # Architecture
//!
//! The wallet handles all key management and transaction signing.
//! Vesl coordinates via the wallet's private gRPC API:
//!
//! 1. **Peek** — query wallet balance and available UTXOs
//! 2. **Poke** — request transaction creation with Vesl settlement context
//!
//! # Transaction Flow
//!
//! ```text
//! Vesl                           Wallet (private gRPC)
//!  |                               |
//!  |-- peek(balance-by-pubkey) --> |  // Check funding
//!  |<-- balance data ------------ |
//!  |                               |
//!  |-- poke(create-tx ...) -----> |  // Request settlement tx
//!  |<-- ack --------------------- |  // Wallet signs + broadcasts
//!  |                               |
//!  |-- (ChainClient) ----------> |  // Monitor acceptance
//!  |   check_accepted(tx_id)      |
//! ```
//!
//! # Signing Limitation (Phase 3.3)
//!
//! The wallet's `create-tx` command constructs outputs from recipient specs
//! but does not currently support attaching custom `NoteData` to output seeds.
//! Phase 3.3 provides the coordination layer and noun construction; full
//! NoteData-in-output support requires either wallet upstream changes or
//! local key management (Phase 5).

use anyhow::Result;
use nockapp::noun::slab::{NockJammer, NounSlab};
use noun_serde::NounEncode;
use nockvm::ext::make_tas;
use nockvm::noun::{Noun, D, T};

use crate::chain::SettlementData;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for wallet coordination.
#[derive(Debug, Clone)]
pub struct WalletConfig {
    /// Private gRPC endpoint of the wallet instance.
    pub endpoint: String,
}

impl WalletConfig {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
        }
    }
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self::new("http://localhost:5555")
    }
}

// ---------------------------------------------------------------------------
// WalletBalance — peek response wrapper
// ---------------------------------------------------------------------------

/// Wallet balance data returned from a peek query.
#[derive(Debug, Clone)]
pub struct WalletBalance {
    /// Raw JAM-encoded balance data from wallet kernel.
    /// Full decoding depends on wallet kernel state format.
    pub raw_data: Vec<u8>,
}

impl std::fmt::Display for WalletBalance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WalletBalance({} bytes)", self.raw_data.len())
    }
}

// ---------------------------------------------------------------------------
// WalletClient — private gRPC client for wallet coordination
// ---------------------------------------------------------------------------

/// Client for coordinating with a nockchain-wallet instance.
///
/// Communicates via the wallet's private gRPC API (peek/poke).
/// The wallet handles all key management and signing internally.
pub struct WalletClient {
    client: nockapp_grpc::private_nockapp::PrivateNockAppGrpcClient,
    config: WalletConfig,
    pid_counter: i32,
}

impl WalletClient {
    /// Connect to a wallet's private gRPC endpoint.
    pub async fn connect(config: WalletConfig) -> Result<Self> {
        let client =
            nockapp_grpc::private_nockapp::PrivateNockAppGrpcClient::connect(&config.endpoint)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to connect to wallet at {}: {e:?}",
                        config.endpoint
                    )
                })?;
        Ok(Self {
            client,
            config,
            pid_counter: 0,
        })
    }

    fn next_pid(&mut self) -> i32 {
        self.pid_counter += 1;
        self.pid_counter
    }

    /// Check if the wallet is running and responsive.
    ///
    /// Attempts a peek; returns `true` if the wallet's gRPC server
    /// responds (even with an application-level error).
    pub async fn check_ready(&mut self) -> Result<bool> {
        let path = build_peek_path(&["show"]);
        let pid = self.next_pid();
        match self.client.peek(pid, path).await {
            Ok(_) => Ok(true),
            Err(nockapp_grpc::NockAppGrpcError::Internal(_)) => {
                // Wallet responded with an error — gRPC server is alive
                Ok(true)
            }
            Err(e) => Err(anyhow::anyhow!("wallet not responsive: {e:?}")),
        }
    }

    /// Query wallet balance by public key (base58).
    ///
    /// Returns raw JAM-encoded balance data from the wallet kernel.
    pub async fn peek_balance(&mut self, pubkey_b58: &str) -> Result<WalletBalance> {
        let path = build_peek_path(&["balance-by-pubkey", pubkey_b58]);
        let pid = self.next_pid();
        let data = self
            .client
            .peek(pid, path)
            .await
            .map_err(|e| anyhow::anyhow!("failed to peek wallet balance: {e:?}"))?;
        Ok(WalletBalance { raw_data: data })
    }

    /// Query wallet balance by note first-name (base58 hash).
    pub async fn peek_balance_by_name(
        &mut self,
        first_name_b58: &str,
    ) -> Result<WalletBalance> {
        let path = build_peek_path(&["balance-by-first-name", first_name_b58]);
        let pid = self.next_pid();
        let data = self
            .client
            .peek(pid, path)
            .await
            .map_err(|e| anyhow::anyhow!("failed to peek wallet balance: {e:?}"))?;
        Ok(WalletBalance { raw_data: data })
    }

    /// Request the wallet to sign a hash.
    ///
    /// Pokes the wallet with `[%sign-hash hash key-index hardened]`.
    /// Returns `true` if the wallet acknowledged the poke.
    ///
    /// # Signing Coordination Flow
    ///
    /// 1. Vesl constructs an unsigned transaction with Vesl NoteData
    /// 2. Vesl computes the transaction hash
    /// 3. Vesl asks the wallet to sign the hash (this method)
    /// 4. Vesl retrieves the signature from wallet effects
    /// 5. Vesl applies the signature and submits via ChainClient
    pub async fn request_sign_hash(
        &mut self,
        hash_b58: &str,
        key_index: u64,
        hardened: bool,
    ) -> Result<bool> {
        let payload = build_sign_hash_poke(hash_b58, key_index, hardened);
        let wire = nockapp_grpc::wire_conversion::create_grpc_wire();
        let pid = self.next_pid();
        self.client
            .poke(pid, wire, payload)
            .await
            .map_err(|e| anyhow::anyhow!("wallet sign-hash failed: {e:?}"))
    }

    /// Request the wallet to create and submit a settlement transaction.
    ///
    /// Pokes the wallet with a `create-tx` command that:
    /// - Spends the specified input UTXO
    /// - Creates an output to the specified recipient
    /// - Uses default signing key (index 0, not hardened)
    ///
    /// The settlement data is logged by the hull for local tracking.
    /// The wallet handles signing and broadcasting internally.
    ///
    /// # Current Limitation
    ///
    /// The wallet's `create-tx` kernel does not support attaching custom
    /// `NoteData` to output seeds. The settlement data is tracked locally
    /// by the hull but not embedded in the on-chain note. Full NoteData
    /// embedding requires upstream wallet changes or local key management
    /// (Phase 5).
    pub async fn request_settlement_tx(
        &mut self,
        input_first: &str,
        input_last: &str,
        recipient_address: &str,
        amount_nicks: u64,
        fee_nicks: u64,
        _settlement: &SettlementData,
    ) -> Result<bool> {
        let payload = build_create_tx_poke(
            input_first,
            input_last,
            recipient_address,
            amount_nicks,
            fee_nicks,
        );
        let wire = nockapp_grpc::wire_conversion::create_grpc_wire();
        let pid = self.next_pid();
        self.client
            .poke(pid, wire, payload)
            .await
            .map_err(|e| anyhow::anyhow!("wallet create-tx failed: {e:?}"))
    }

    /// Get the wallet config.
    pub fn config(&self) -> &WalletConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// Noun construction — free functions for testability
// ---------------------------------------------------------------------------

/// Build a JAM-encoded peek path from string segments.
///
/// The wallet kernel expects peek paths as Nock lists of cord atoms.
/// `["balance-by-pubkey", "abc123"]` becomes the noun `[%balance-by-pubkey %abc123 ~]`.
pub fn build_peek_path(segments: &[&str]) -> Vec<u8> {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let path: Vec<String> = segments.iter().map(|s| s.to_string()).collect();
    let noun = path.to_noun(&mut slab);
    slab.set_root(noun);
    slab.jam().to_vec()
}

/// Build a JAM-encoded `sign-hash` poke payload.
///
/// Noun format: `[%sign-hash hash-cord index-atom hardened-loobean]`
///
/// The wallet kernel signs the provided hash with the key at the given
/// derivation index. Hardened derivation uses a different key path.
pub fn build_sign_hash_poke(hash_b58: &str, key_index: u64, hardened: bool) -> Vec<u8> {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let tag = make_tas(&mut slab, "sign-hash").as_noun();
    let hash = make_tas(&mut slab, hash_b58).as_noun();
    let index = D(key_index);
    // Hoon loobean: %.y = 0 (true), %.n = 1 (false)
    let hard: Noun = if hardened { D(0) } else { D(1) };
    let cmd = T(&mut slab, &[tag, hash, index, hard]);
    slab.set_root(cmd);
    slab.jam().to_vec()
}

/// Build a JAM-encoded `create-tx` poke payload.
///
/// Matches the wallet CLI's `create_tx` noun structure:
/// ```text
/// [%create-tx
///   names=[[first last] ~]        :: input UTXOs to spend
///   order=[[amount address] ~]    :: output recipients
///   fee=@ud                       :: miner fee in nicks
///   allow-low-fee=%.n             :: don't allow below-minimum fee
///   refund-pkh=~                  :: no explicit refund address
///   sign-keys=[[0 %.n] ~]         :: sign with key index 0, not hardened
///   include-data=%.n              :: don't forward input note data
///   save-raw-tx=%.n               :: don't save tx to file
///   note-selection=%auto          :: automatic UTXO selection
/// ]
/// ```
///
/// The exact noun format is derived from the wallet CLI source at
/// `$NOCK_HOME/crates/nockchain-wallet/src/main.rs:create_tx()`.
/// Validation against a live wallet kernel happens during Phase 3.4
/// (fakenet testing).
pub fn build_create_tx_poke(
    input_first: &str,
    input_last: &str,
    recipient_address: &str,
    amount_nicks: u64,
    fee_nicks: u64,
) -> Vec<u8> {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();

    let tag = make_tas(&mut slab, "create-tx").as_noun();

    // names: list of [first last] pairs — the UTXOs to spend
    let first = make_tas(&mut slab, input_first).as_noun();
    let last = make_tas(&mut slab, input_last).as_noun();
    let name_pair = T(&mut slab, &[first, last]);
    let names = T(&mut slab, &[name_pair, D(0)]); // [pair ~]

    // order: list of [amount address] pairs — outputs to create
    let amt = D(amount_nicks);
    let addr = make_tas(&mut slab, recipient_address).as_noun();
    let recipient_pair = T(&mut slab, &[amt, addr]);
    let order = T(&mut slab, &[recipient_pair, D(0)]); // [pair ~]

    // fee
    let fee = D(fee_nicks);

    // allow-low-fee: %.n (false)
    let allow_low_fee = D(1);

    // refund-pkh: ~ (null)
    let refund = D(0);

    // sign-keys: [[0 %.n] ~] — sign with key index 0, not hardened
    let key_pair = T(&mut slab, &[D(0), D(1)]);
    let sign_keys = T(&mut slab, &[key_pair, D(0)]);

    // include-data: %.n
    let include_data = D(1);

    // save-raw-tx: %.n
    let save_raw = D(1);

    // note-selection: %auto
    let note_sel = make_tas(&mut slab, "auto").as_noun();

    let cmd = T(
        &mut slab,
        &[
            tag,
            names,
            order,
            fee,
            allow_low_fee,
            refund,
            sign_keys,
            include_data,
            save_raw,
            note_sel,
        ],
    );
    slab.set_root(cmd);
    slab.jam().to_vec()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_config_defaults() {
        let cfg = WalletConfig::default();
        assert_eq!(cfg.endpoint, "http://localhost:5555");
    }

    #[test]
    fn wallet_config_custom() {
        let cfg = WalletConfig::new("http://wallet:6666");
        assert_eq!(cfg.endpoint, "http://wallet:6666");
    }

    #[test]
    fn peek_path_produces_nonempty_jam() {
        let path = build_peek_path(&["balance-by-pubkey", "abc123"]);
        assert!(!path.is_empty(), "jammed peek path must not be empty");
    }

    #[test]
    fn peek_path_single_segment() {
        let path = build_peek_path(&["show"]);
        assert!(!path.is_empty());
    }

    #[test]
    fn peek_path_deterministic() {
        let p1 = build_peek_path(&["balance-by-pubkey", "key1"]);
        let p2 = build_peek_path(&["balance-by-pubkey", "key1"]);
        assert_eq!(p1, p2, "same segments must produce identical JAM bytes");
    }

    #[test]
    fn peek_path_varies_with_input() {
        let p1 = build_peek_path(&["balance-by-pubkey", "key1"]);
        let p2 = build_peek_path(&["balance-by-pubkey", "key2"]);
        assert_ne!(p1, p2, "different keys must produce different JAM bytes");
    }

    #[test]
    fn sign_hash_poke_nonempty() {
        let payload = build_sign_hash_poke("somehash", 0, false);
        assert!(!payload.is_empty());
    }

    #[test]
    fn sign_hash_poke_deterministic() {
        let p1 = build_sign_hash_poke("hash1", 3, true);
        let p2 = build_sign_hash_poke("hash1", 3, true);
        assert_eq!(p1, p2);
    }

    #[test]
    fn sign_hash_poke_varies_with_hardened() {
        let p1 = build_sign_hash_poke("hash1", 0, false);
        let p2 = build_sign_hash_poke("hash1", 0, true);
        assert_ne!(p1, p2, "hardened flag must change the jammed output");
    }

    #[test]
    fn create_tx_poke_nonempty() {
        let payload = build_create_tx_poke("first", "last", "recipient", 100_000, 1_000);
        assert!(!payload.is_empty());
    }

    #[test]
    fn create_tx_poke_deterministic() {
        let p1 = build_create_tx_poke("f", "l", "r", 100, 10);
        let p2 = build_create_tx_poke("f", "l", "r", 100, 10);
        assert_eq!(p1, p2);
    }

    #[test]
    fn create_tx_poke_varies_with_amount() {
        let p1 = build_create_tx_poke("f", "l", "r", 100, 10);
        let p2 = build_create_tx_poke("f", "l", "r", 200, 10);
        assert_ne!(p1, p2, "different amounts must produce different payloads");
    }

    #[test]
    fn create_tx_poke_varies_with_recipient() {
        let p1 = build_create_tx_poke("f", "l", "addr1", 100, 10);
        let p2 = build_create_tx_poke("f", "l", "addr2", 100, 10);
        assert_ne!(p1, p2);
    }

    #[test]
    fn wallet_balance_display() {
        let bal = WalletBalance {
            raw_data: vec![1, 2, 3],
        };
        let s = format!("{bal}");
        assert!(s.contains("3 bytes"));
    }
}
