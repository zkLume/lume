//! Lume Vessel — library target for integration tests.
//!
//! Re-exports internal modules so that `tests/e2e_fakenet.rs` and other
//! integration tests can reference `vessel::chain`, `vessel::wallet`, etc.
//!
//! The binary entry point remains in `main.rs`.

pub mod api;
pub mod chain;
pub mod ingest;
pub mod llm;
pub mod merkle;
pub mod noun_builder;
pub mod retrieve;
pub mod types;
pub mod wallet;
pub mod wallet_kernel;
