//! In-process wallet kernel integration — boots wal.jam as a NockApp.
//!
//! Instead of shelling out to the `nockchain-wallet` CLI (which boots,
//! runs one command, and exits), this module boots the wallet kernel
//! directly in-process using the same `boot::setup()` mechanism as
//! the Vesl kernel tests.
//!
//! # Architecture
//!
//! ```text
//! WalletKernel
//!   └─ NockApp (boots wal.jam)
//!       ├─ poke [%keygen entropy salt]         → generates keys
//!       ├─ poke [%import-seed-phrase phrase v]  → imports known test keys
//!       ├─ poke [%fakenet constants]            → sets fakenet mode
//!       ├─ peek [%signing-keys ~]              → returns [Hash] (PKHs)
//!       └─ peek [%state ~]                     → returns full wallet state
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! let mut wk = WalletKernel::boot(tmp.path()).await?;
//! wk.import_seed_phrase(SEED, 1).await?;
//! wk.set_fakenet().await?;
//! let keys = wk.peek_signing_keys().await?;
//! ```
//!
//! # Long-term Value
//!
//! This is the foundation for STRATEGY.md's B2 (`nockchain-client-rs`)
//! and Phase 5 local key management. Every NockApp that needs wallet
//! operations (keygen, signing, tx creation) can use this pattern
//! instead of depending on the CLI binary.

use std::path::Path;

use anyhow::Result;
use clap::Parser;
use nockapp::kernel::boot;
use nockapp::noun::slab::{NockJammer, NounSlab};
use nockapp::utils::bytes::Byts;
use nockapp::wire::{SystemWire, Wire, WireRepr};
use nockapp::NockApp;
use nockchain_math::belt::Belt;
use nockchain_types::tx_engine::common::{
    BlockHeight, Hash as ChainHash, Name as ChainName, Nicks,
};
use nockchain_types::tx_engine::v1::note::{
    Balance as ChainBalance, BalanceUpdate, Note as ChainNote, NoteData as ChainNoteData,
    NoteDataEntry as ChainNoteDataEntry, NoteV1,
};
use nockchain_types::tx_engine::v1::tx::{Lock, LockPrimitive, Pkh, SpendCondition};
use nockvm::ext::make_tas;
use nockvm::jets::cold::Nounable;
use nockvm::noun::{D, SIG, T};
use noun_serde::prelude::*;

// ---------------------------------------------------------------------------
// WalletKernel — in-process wallet NockApp
// ---------------------------------------------------------------------------

/// In-process wallet kernel for key management and transaction signing.
///
/// Wraps a `NockApp` booted from the wallet kernel JAM (`wal.jam`).
/// Provides typed methods for keygen, seed import, balance peeks,
/// and fakenet configuration.
pub struct WalletKernel {
    app: NockApp,
}

impl WalletKernel {
    /// Boot a fresh wallet kernel in the given data directory.
    ///
    /// Uses `--new` to bypass any cached state.
    /// The `kernel_bytes` parameter should be `kernels_open_wallet::KERNEL`.
    pub async fn boot(kernel_bytes: &[u8], data_dir: &Path) -> Result<Self> {
        let cli = boot::Cli::parse_from(["wallet", "--new"]);
        let app: NockApp = boot::setup(
            kernel_bytes,
            cli,
            &[], // no hot state needed for keygen/import
            "wallet",
            Some(data_dir.to_path_buf()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("failed to boot wallet kernel: {e}"))?;
        Ok(Self { app })
    }

    /// Generate a new keypair from random entropy.
    ///
    /// Pokes `[%keygen entropy salt]` where entropy is 32 random bytes
    /// and salt is 16 random bytes.
    pub async fn keygen(&mut self) -> Result<()> {
        let mut entropy = [0u8; 32];
        let mut salt = [0u8; 16];
        getrandom::fill(&mut entropy)
            .map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
        getrandom::fill(&mut salt)
            .map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;

        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tas(&mut slab, "keygen").as_noun();
        let ent_noun = Byts(entropy.to_vec()).into_noun(&mut slab);
        let sal_noun = Byts(salt.to_vec()).into_noun(&mut slab);
        let cmd = T(&mut slab, &[tag, ent_noun, sal_noun]);
        slab.set_root(cmd);

        let _effects = self
            .app
            .poke(SystemWire.to_wire(), slab)
            .await
            .map_err(|e| anyhow::anyhow!("wallet keygen poke failed: {e:?}"))?;
        Ok(())
    }

    /// Import a known seed phrase for reproducible test keys.
    ///
    /// Pokes `[%import-seed-phrase phrase version]`.
    pub async fn import_seed_phrase(&mut self, phrase: &str, version: u64) -> Result<()> {
        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tas(&mut slab, "import-seed-phrase").as_noun();
        let phrase_noun = make_tas(&mut slab, phrase).as_noun();
        let version_noun = D(version);
        let cmd = T(&mut slab, &[tag, phrase_noun, version_noun]);
        slab.set_root(cmd);

        let _effects = self
            .app
            .poke(SystemWire.to_wire(), slab)
            .await
            .map_err(|e| anyhow::anyhow!("wallet import-seed-phrase poke failed: {e:?}"))?;
        Ok(())
    }

    /// Set the wallet to fakenet mode with default fakenet blockchain constants.
    ///
    /// Pokes `[%fakenet constants]` where constants are the default
    /// fakenet blockchain constants (coinbase_timelock_min=1, etc.).
    pub async fn set_fakenet(&mut self) -> Result<()> {
        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tas(&mut slab, "fakenet").as_noun();
        let constants = nockchain_types::default_fakenet_blockchain_constants();
        let constants_noun = constants.to_noun(&mut slab);
        let cmd = T(&mut slab, &[tag, constants_noun]);
        slab.set_root(cmd);

        let _effects = self
            .app
            .poke(SystemWire.to_wire(), slab)
            .await
            .map_err(|e| anyhow::anyhow!("wallet fakenet poke failed: {e:?}"))?;
        Ok(())
    }

    /// Peek the wallet's signing keys (PKH hashes).
    ///
    /// Returns the list of `Hash` values (tip5 digests of public keys)
    /// that the wallet can sign with.
    pub async fn peek_signing_keys(&mut self) -> Result<Vec<ChainHash>> {
        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tas(&mut slab, "signing-keys").as_noun();
        let path = T(&mut slab, &[tag, SIG]);
        slab.set_root(path);

        let result = self
            .app
            .peek(slab)
            .await
            .map_err(|e| anyhow::anyhow!("wallet signing-keys peek failed: {e:?}"))?;

        // SAFETY: result is a NounSlab returned from NockApp::peek. The root
        // is valid while the slab is live. from_noun reads but does not take ownership.
        let decoded: Option<Option<Vec<ChainHash>>> =
            unsafe { Option::from_noun(result.root())? };
        Ok(decoded.flatten().unwrap_or_default())
    }

    /// Peek the wallet's tracked pubkey strings (base58).
    ///
    /// Returns base58-encoded strings: full pubkeys for v0 keys,
    /// PKH hashes for v1 keys.
    pub async fn peek_tracked_pubkeys(&mut self) -> Result<Vec<String>> {
        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tas(&mut slab, "tracked-pubkeys").as_noun();
        let path = T(&mut slab, &[tag, SIG]);
        slab.set_root(path);

        let result = self
            .app
            .peek(slab)
            .await
            .map_err(|e| anyhow::anyhow!("wallet tracked-pubkeys peek failed: {e:?}"))?;

        // SAFETY: result is a NounSlab returned from NockApp::peek. The root
        // is valid while the slab is live. from_noun reads but does not take ownership.
        let decoded: Option<Option<Vec<String>>> =
            unsafe { Option::from_noun(result.root())? };
        Ok(decoded.flatten().unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Balance + Transaction helpers (Phase 3.5.2a)
// ---------------------------------------------------------------------------

/// Jam a Lock into the note-data blob format expected by the wallet kernel.
///
/// The wallet stores locks as NoteData entries with key `"lock"` and
/// value `jam([%0 lock])`. This matches `wallet-tx-builder`'s
/// `TypedNoteDataEntry::Lock` encoding.
fn jam_lock_to_blob(lock: &Lock) -> bytes::Bytes {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let version_tag = D(0);
    let lock_noun = lock.to_noun(&mut slab);
    let payload = T(&mut slab, &[version_tag, lock_noun]);
    slab.set_root(payload);
    slab.jam()
}

/// Create a simple 1-of-1 PKH lock (single signer, no timelock).
pub fn simple_pkh_lock(pkh: ChainHash) -> Lock {
    Lock::SpendCondition(SpendCondition::new(vec![LockPrimitive::Pkh(Pkh::new(
        1,
        vec![pkh],
    ))]))
}

/// Create a NoteV1 with a lock encoded in its note_data.
///
/// Replicates the wallet test suite's `note_v1_with_lock()` pattern.
pub fn note_v1_with_lock(
    name: ChainName,
    origin_page: u64,
    assets: u64,
    lock: Lock,
) -> ChainNote {
    let lock_blob = jam_lock_to_blob(&lock);
    let note_data =
        ChainNoteData::new(vec![ChainNoteDataEntry::new("lock".to_string(), lock_blob)]);
    ChainNote::V1(NoteV1::new(
        BlockHeight(Belt(origin_page)),
        name,
        note_data,
        Nicks(assets as usize),
    ))
}

/// Create a BalanceUpdate from test data.
///
/// Replicates the wallet test suite's `balance_page()` pattern.
pub fn balance_page(
    height: u64,
    block_id_val: u64,
    notes: Vec<(ChainName, ChainNote)>,
) -> BalanceUpdate {
    BalanceUpdate {
        height: BlockHeight(Belt(height)),
        block_id: ChainHash::from_limbs(&[block_id_val, 0, 0, 0, 0]),
        notes: ChainBalance(notes),
    }
}

impl WalletKernel {
    /// Feed a balance update to the wallet kernel so it knows about UTXOs.
    ///
    /// Constructs `[%update-balance-grpc Some(Some(BalanceUpdate))]` and
    /// pokes the kernel. After this, the kernel can spend notes from the
    /// provided balance in `create_tx_p2pkh`.
    pub async fn apply_balance_update(&mut self, balance: BalanceUpdate) -> Result<()> {
        let mut slab: NounSlab = NounSlab::new();
        let wrapped: Option<Option<BalanceUpdate>> = Some(Some(balance));
        let payload_noun = wrapped.to_noun(&mut slab);
        let tag = make_tas(&mut slab, "update-balance-grpc").as_noun();
        let cmd = T(&mut slab, &[tag, payload_noun]);
        slab.set_root(cmd);

        let _effects = self
            .app
            .poke(SystemWire.to_wire(), slab)
            .await
            .map_err(|e| anyhow::anyhow!("balance update poke failed: {e:?}"))?;
        Ok(())
    }

    /// Create a P2PKH self-send transaction.
    ///
    /// Pokes the wallet kernel with `%create-tx` using the provided input
    /// note names, recipient PKH, amount, and fee. Returns the raw effects
    /// from the kernel (which include `[%file %write ...]` with the jammed tx).
    ///
    /// The noun format matches the wallet CLI's `create_tx()` in create_tx.rs.
    pub async fn create_tx_p2pkh(
        &mut self,
        input_first_b58: &str,
        input_last_b58: &str,
        recipient_pkh: &ChainHash,
        amount: u64,
        fee: u64,
    ) -> Result<Vec<NounSlab>> {
        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tas(&mut slab, "create-tx").as_noun();

        // names: [[first last] ~] — base58-encoded name hashes as text atoms
        let first = make_tas(&mut slab, input_first_b58).as_noun();
        let last = make_tas(&mut slab, input_last_b58).as_noun();
        let name_pair = T(&mut slab, &[first, last]);
        let names = T(&mut slab, &[name_pair, D(0)]); // one-element list

        // orders: [[%pkh recipient_hash amount] ~]
        let pkh_tag = make_tas(&mut slab, "pkh").as_noun();
        let recipient_noun = recipient_pkh.to_noun(&mut slab);
        let order = T(&mut slab, &[pkh_tag, recipient_noun, D(amount)]);
        let orders = T(&mut slab, &[order, D(0)]); // one-element list

        // fee: coins (u64 atom)
        let fee_noun = D(fee);

        // allow-low-fee: %.y (true = 0, for fakenet testing)
        let allow_low_fee = D(0);

        // sign-keys: ~ (null — let the kernel auto-select signing key)
        // The wallet kernel picks the default key when sign-keys is null.
        let sign_keys = SIG;

        // refund-pkh: ~ (null — no explicit refund address)
        let refund = D(0);

        // include-data: %.y (yes, include note data)
        let include_data = D(0);

        // save-raw-tx: %.n (no debug files)
        let save_raw = D(1);

        // selection-strategy: %asc (ascending note selection)
        let selection = make_tas(&mut slab, "asc").as_noun();

        let cmd = T(
            &mut slab,
            &[
                tag,
                names,
                orders,
                fee_noun,
                allow_low_fee,
                sign_keys,
                refund,
                include_data,
                save_raw,
                selection,
            ],
        );
        slab.set_root(cmd);

        // The wallet kernel dispatches %create-tx on the command wire,
        // NOT on the system wire. Replicate WalletWire::Command(CreateTx).
        let wire = WireRepr::new(
            "wallet",
            1,
            vec!["command".into(), "create-tx".into()],
        );

        let effects = self
            .app
            .poke(wire, slab)
            .await
            .map_err(|e| anyhow::anyhow!("create-tx poke failed: {e:?}"))?;
        Ok(effects)
    }

    /// Peek the wallet balance state.
    ///
    /// Returns the raw NounSlab from the peek (caller must decode).
    pub async fn peek_balance(&mut self) -> Result<NounSlab> {
        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tas(&mut slab, "balance").as_noun();
        let path = T(&mut slab, &[tag, SIG]);
        slab.set_root(path);

        self.app
            .peek(slab)
            .await
            .map_err(|e| anyhow::anyhow!("wallet balance peek failed: {e:?}"))
    }
}

// ---------------------------------------------------------------------------
// Test seed phrase (from wallet test suite)
// ---------------------------------------------------------------------------

/// A known test seed phrase from the wallet's own test suite.
/// Produces deterministic keys for reproducible testing.
pub const TEST_SEED_PHRASE: &str = "route run sing warrior light swamp clog flower agent ugly wasp fresh tube snow motion salt salon village raccoon chair demise neutral school confirm";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Wallet kernel tests require the kernels-open-wallet crate,
    // which is a dev-dependency. They're gated behind #[ignore]
    // since booting the 20MB wallet kernel takes several seconds.

    #[cfg(feature = "_wallet_kernel_tests")]
    mod wallet_kernel_tests {
        use super::*;

        #[tokio::test]
        async fn wallet_kernel_boots() {
            let tmp = tempfile::tempdir().unwrap();
            let wk = WalletKernel::boot(
                kernels_open_wallet::KERNEL,
                tmp.path(),
            )
            .await;
            assert!(wk.is_ok(), "wallet kernel must boot: {:?}", wk.err());
        }

        #[tokio::test]
        async fn wallet_kernel_import_and_peek_keys() {
            let tmp = tempfile::tempdir().unwrap();
            let mut wk = WalletKernel::boot(
                kernels_open_wallet::KERNEL,
                tmp.path(),
            )
            .await
            .expect("boot");

            wk.import_seed_phrase(TEST_SEED_PHRASE, 1)
                .await
                .expect("import");

            let keys = wk.peek_signing_keys().await.expect("peek");
            assert!(
                !keys.is_empty(),
                "wallet must have at least one signing key after import"
            );
            println!("Signing keys: {:?}", keys.iter().map(|k| k.to_base58()).collect::<Vec<_>>());
        }
    }
}
