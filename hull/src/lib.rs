//! Vesl Hull — library target for integration tests.
//!
//! Re-exports internal modules so that `tests/e2e_fakenet.rs` and other
//! integration tests can reference `hull::chain`, `hull::wallet`, etc.
//!
//! The binary entry point remains in `main.rs`.

pub mod api;
pub mod chain;
pub mod ingest;
pub mod llm;
pub mod merkle;
pub mod noun_builder;
pub mod retrieve;
pub mod signing;
pub mod tx_builder;
pub mod types;
pub mod wallet;
pub mod wallet_kernel;
