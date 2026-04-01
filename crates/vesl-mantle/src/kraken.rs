//! Kraken — STARK Proof (heaviest tentacle)
//!
//! Everything Beak does, plus STARK proof generation.
//! Pokes kernel with %prove instead of %settle,
//! gets back a settled note + STARK proof bytes.

use anyhow::Result;

use nockchain_tip5_rs::Tip5Hash;

use crate::types::{Manifest, Note};

pub struct Kraken {
    // Internal: Beak + prover hot state (zkvm-jetpack).
    // Entirely stubbed — STARK prover wiring is the heaviest lift.
}

impl Kraken {
    /// Boot with STARK prover capabilities.
    ///
    /// Loads kraken.jam (full kernel with prover) and zkvm-jetpack hot state.
    pub async fn boot_with_stark() -> Result<Self> {
        todo!("boot_with_stark: load kraken.jam, init zkvm-jetpack prover")
    }

    /// Settle with STARK proof generation.
    ///
    /// Pokes kernel with %prove instead of %settle. Returns the settled
    /// note and the STARK proof bytes.
    pub async fn prove_and_settle(
        &mut self,
        _note: &Note,
        _manifest: &Manifest,
        _root: &Tip5Hash,
    ) -> Result<(Note, Vec<u8>)> {
        todo!("prove_and_settle: kernel %prove poke → (note, stark-proof)")
    }
}
