//! In-process wallet kernel integration — boots wal.jam as a NockApp.
//!
//! Instead of shelling out to the `nockchain-wallet` CLI (which boots,
//! runs one command, and exits), this module boots the wallet kernel
//! directly in-process using the same `boot::setup()` mechanism as
//! the Lume kernel tests.
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
use nockapp::noun::slab::NounSlab;
use nockapp::utils::bytes::Byts;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use nockchain_types::tx_engine::common::Hash as ChainHash;
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

        let decoded: Option<Option<Vec<String>>> =
            unsafe { Option::from_noun(result.root())? };
        Ok(decoded.flatten().unwrap_or_default())
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
