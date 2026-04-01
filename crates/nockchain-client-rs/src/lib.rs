//! Nockchain gRPC client for NockApp developers.
//!
//! Provides everything you need to talk to a Nockchain node and wallet
//! from Rust: transaction submission, balance queries, wallet signing
//! coordination, and NoteData encoding/decoding.
//!
//! # Quick Start
//!
//! ```ignore
//! use nockchain_client_rs::{ChainClient, ChainConfig, WalletClient, WalletConfig};
//!
//! // Connect to a local Nockchain node
//! let mut chain = ChainClient::connect(ChainConfig::default()).await?;
//!
//! // Query balance
//! let balance = chain.get_balance("your-address-base58").await?;
//!
//! // Connect to wallet for signing
//! let mut wallet = WalletClient::connect(WalletConfig::default()).await?;
//! ```
//!
//! # Architecture
//!
//! Two gRPC clients, two protocols:
//!
//! - **ChainClient** talks to the Nockchain node's *public* gRPC API
//!   (default port 9090). Submits transactions, queries balances,
//!   polls for acceptance.
//!
//! - **WalletClient** talks to a nockchain-wallet's *private* gRPC API
//!   (default port 5555). Requests signing, queries wallet state,
//!   creates transactions.
//!
//! NoteData helpers encode and decode the key-value entries that ride
//! inside NoteV1 transactions. Every NockApp that puts data on-chain
//! needs these.

pub mod chain;
pub mod note_data;
pub mod types;
pub mod wallet;

// Re-export the main types at crate root for convenience.
pub use chain::{ChainClient, ChainConfig};
pub use note_data::{find_entry, find_hash_entry, find_u64_entry, jam_tip5_entry, jam_u64_entry};
pub use types::{extract_note_data, extract_spendable_utxos, SpendableUtxo};
pub use wallet::{WalletClient, WalletConfig};

// Re-export nockchain types that callers will need.
pub use nockchain_tip5_rs::Tip5Hash;
pub use nockchain_types::tx_engine::common::Hash as ChainHash;
pub use nockchain_types::tx_engine::v1::note::{NoteData, NoteDataEntry};
pub use nockchain_types::tx_engine::v1::RawTx;
