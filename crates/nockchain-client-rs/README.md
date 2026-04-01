# nockchain-client-rs

You need to talk to the chain. You need to talk to a wallet. You shouldn't have to reverse-engineer two gRPC protocols and a noun serialization format to do it.

## What This Is

A Rust crate that wraps the two gRPC APIs every NockApp developer needs:

1. **ChainClient** — public gRPC (port 9090): submit transactions, poll for acceptance, query balances
2. **WalletClient** — private gRPC (port 5555): request signing, create transactions, peek wallet state

Plus **NoteData helpers** for encoding/decoding the key-value entries that ride inside NoteV1 transactions. If your NockApp puts data on-chain, you need these.

## Quick Start

```rust
use nockchain_client_rs::{ChainClient, ChainConfig, WalletClient, WalletConfig};

// Connect to a local Nockchain node
let mut chain = ChainClient::connect(ChainConfig::default()).await?;

// Check balance
let balance = chain.get_balance_by_pkh("your-pkh-base58", 1).await?;

// Submit a transaction and wait for block inclusion
let accepted = chain.submit_and_wait(raw_tx, "tx-id-base58").await?;

// Connect to wallet for signing
let mut wallet = WalletClient::connect(WalletConfig::default()).await?;
wallet.request_sign_hash("tx-hash-base58", 0, false).await?;
```

## NoteData Encoding

NoteV1 transactions carry `NoteData` — a list of key-value entries where values are JAM-encoded Nock nouns. This crate provides type-safe encode/decode helpers:

```rust
use nockchain_client_rs::{jam_u64_entry, jam_tip5_entry, find_u64_entry, find_hash_entry};
use nockchain_client_rs::NoteData;

// Encode
let entries = vec![
    jam_u64_entry("my-app-version", 1),
    jam_u64_entry("my-app-id", 42),
    jam_tip5_entry("my-app-root", &merkle_root),
];
let note_data = NoteData::new(entries);

// Decode
let version = find_u64_entry(&note_data, "my-app-version")?;
let root = find_hash_entry(&note_data, "my-app-root")?;
```

## Balance Queries

Three ways to query, depending on what you have:

```rust
// Full SchnorrPubkey (97 bytes base58)
let bal = chain.get_balance("full-pubkey-base58").await?;

// PKH hash — tries coinbase FirstName, then simple P2PKH
let bal = chain.get_balance_by_pkh("pkh-base58", 1).await?;

// Unknown format — tries Address, falls back to FirstName
let bal = chain.get_balance_flexible("something-base58").await?;
```

Extract UTXOs from a balance response:

```rust
use nockchain_client_rs::extract_spendable_utxos;

let utxos = extract_spendable_utxos(&balance);
for utxo in &utxos {
    println!("{}: {} nicks", utxo.first_name().to_base58(), utxo.amount);
    if let Some(note_data) = &utxo.note_data {
        // Decode your app-specific data from the NoteData entries
    }
}
```

## Wallet Coordination

The wallet client uses peek (read) and poke (write) to coordinate:

```rust
let mut wallet = WalletClient::connect(WalletConfig::default()).await?;

// Check wallet is alive
let ready = wallet.check_ready().await?;

// Query balance by pubkey
let bal = wallet.peek_balance("pubkey-base58").await?;

// Request transaction creation
wallet.request_create_tx(
    "input-first-name",
    "input-last-name",
    "recipient-address",
    100_000,  // amount in nicks
    1_000,    // fee in nicks
).await?;
```

## What's Not Here

This crate is the generic chain interaction layer. App-specific logic (like Vesl's settlement verification, Merkle proofs, STARK proving) lives in the app, not here.

## Dependencies

Depends on the nockchain monorepo crates via local paths. For standalone use, switch to git dependencies:

```toml
[dependencies]
nockchain-client-rs = { git = "https://github.com/zkVesl/vesl.git", path = "crates/nockchain-client-rs" }
```

~
