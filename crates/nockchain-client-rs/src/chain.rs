//! Nockchain public gRPC client — transaction submission, balance queries, polling.
//!
//! Wraps `PublicNockchainGrpcClient` with ergonomic methods for the operations
//! every NockApp needs: submit a transaction, wait for it to land in a block,
//! and query balances.
//!
//! # Usage
//!
//! ```ignore
//! let mut client = ChainClient::connect(ChainConfig::default()).await?;
//!
//! // Submit and wait for block inclusion
//! let accepted = client.submit_and_wait(raw_tx, "tx-id-base58").await?;
//!
//! // Query balance by address or PKH
//! let balance = client.get_balance_by_pkh("pkh-base58", 1).await?;
//! ```

use std::time::Duration;

use anyhow::Result;
use nockchain_types::tx_engine::common::Hash as ChainHash;
use nockchain_types::tx_engine::v1::tx::SpendCondition;

// ---------------------------------------------------------------------------
// Configuration
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
    ///
    /// Defaults: 5s poll interval, 120s timeout.
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
// ChainClient
// ---------------------------------------------------------------------------

/// Client for interacting with a Nockchain node's public gRPC API.
///
/// Provides methods for:
/// - Submitting pre-signed transactions
/// - Polling for transaction acceptance (block inclusion)
/// - Querying balances by address, PKH, or FirstName
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
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to connect to Nockchain gRPC at {}: {e:?}",
                    config.endpoint
                )
            })?;
        Ok(Self { client, config })
    }

    /// Submit a pre-signed raw transaction to the Nockchain node.
    ///
    /// Returns `Ok(())` on acknowledgment. The transaction is not yet in a
    /// block — call [`wait_for_acceptance`] to confirm inclusion.
    pub async fn submit_transaction(
        &mut self,
        raw_tx: nockchain_types::tx_engine::v1::RawTx,
    ) -> Result<()> {
        self.client
            .wallet_send_transaction(raw_tx)
            .await
            .map_err(|e| anyhow::anyhow!("failed to submit transaction: {e:?}"))?;
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
                    eprintln!("  warn: check_accepted error (will retry): {}", e);
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
    /// Combines `submit_transaction` + `wait_for_acceptance`.
    /// Returns `true` if the transaction was accepted before timeout.
    pub async fn submit_and_wait(
        &mut self,
        raw_tx: nockchain_types::tx_engine::v1::RawTx,
        tx_id_base58: &str,
    ) -> Result<bool> {
        self.submit_transaction(raw_tx).await?;
        self.wait_for_acceptance(tx_id_base58).await
    }

    /// Get balance by full SchnorrPubkey address (base58, 97 bytes decoded).
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
    /// The Address selector requires a full SchnorrPubkey (97 bytes base58).
    /// If that fails (e.g., input is a PKH hash), falls back to FirstName.
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
            Err(_) => self
                .client
                .wallet_get_balance(&BalanceRequest::FirstName(address.to_string()))
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to get balance for '{}' (tried Address and FirstName): {e:?}",
                        address
                    )
                }),
        }
    }

    /// Get balance by PKH using FirstName computation.
    ///
    /// Computes the note FirstName from the PKH + spend condition, then
    /// queries via `BalanceRequest::FirstName`. Tries coinbase FirstName
    /// first (mining rewards have a timelock), then simple P2PKH.
    pub async fn get_balance_by_pkh(
        &mut self,
        pkh_b58: &str,
        coinbase_timelock_min: u64,
    ) -> Result<nockapp_grpc::pb::common::v2::Balance> {
        use nockapp_grpc::services::public_nockchain::v2::client::BalanceRequest;

        let coinbase_fn = compute_coinbase_first_name(pkh_b58, coinbase_timelock_min)?;
        let simple_fn = compute_simple_first_name(pkh_b58)?;

        match self
            .client
            .wallet_get_balance(&BalanceRequest::FirstName(coinbase_fn.clone()))
            .await
        {
            Ok(bal) if !bal.notes.is_empty() => Ok(bal),
            _ => self
                .client
                .wallet_get_balance(&BalanceRequest::FirstName(simple_fn))
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to get balance by PKH '{}' (tried coinbase and simple): {e:?}",
                        pkh_b58
                    )
                }),
        }
    }

    /// Get the underlying config.
    pub fn config(&self) -> &ChainConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// FirstName computation — derive note FirstName from a PKH
// ---------------------------------------------------------------------------

/// Compute the FirstName for coinbase (mining reward) notes at a given PKH.
///
/// Coinbase notes have a P2PKH lock + relative timelock. The FirstName is
/// the hash of the lock root, which includes both the PKH and the timelock.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    /// Known-good PKH from the Nockchain fakenet.
    const TEST_MINING_PKH: &str = "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV";

    #[test]
    fn coinbase_first_name_computes_from_pkh() {
        let fn_str = compute_coinbase_first_name(TEST_MINING_PKH, 1)
            .expect("coinbase first_name should compute from valid PKH");
        assert!(!fn_str.is_empty());
    }

    #[test]
    fn simple_first_name_computes_from_pkh() {
        let fn_str = compute_simple_first_name(TEST_MINING_PKH)
            .expect("simple first_name should compute from valid PKH");
        assert!(!fn_str.is_empty());
    }

    #[test]
    fn first_names_differ_by_type() {
        let coinbase = compute_coinbase_first_name(TEST_MINING_PKH, 1).unwrap();
        let simple = compute_simple_first_name(TEST_MINING_PKH).unwrap();
        assert_ne!(
            coinbase, simple,
            "coinbase and simple first_names must differ"
        );
    }

    #[test]
    fn coinbase_first_name_rejects_invalid_pkh() {
        let result = compute_coinbase_first_name("not-valid-base58!!!", 1);
        assert!(result.is_err());
    }
}
