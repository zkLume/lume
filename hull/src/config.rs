//! Settlement mode configuration.
//!
//! Centralizes the three settlement modes (local, fakenet, dumbnet) and
//! the resolution logic: CLI flags > env vars > vesl.toml > mode defaults.

use std::fmt;
use std::path::Path;

use clap::ValueEnum;
use nockchain_math::belt::Belt;
use serde::Deserialize;

use crate::signing;

// ---------------------------------------------------------------------------
// Settlement mode enum
// ---------------------------------------------------------------------------

/// Settlement mode — determines how (or whether) the hull interacts with
/// the Nockchain ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SettlementMode {
    /// No chain interaction. Development and unit testing.
    Local,
    /// Localhost fakenet with the demo signing key.
    Fakenet,
    /// Live network with real keys from wallet init.
    Dumbnet,
}

impl fmt::Display for SettlementMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Fakenet => write!(f, "fakenet"),
            Self::Dumbnet => write!(f, "dumbnet"),
        }
    }
}

impl std::str::FromStr for SettlementMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "fakenet" => Ok(Self::Fakenet),
            "dumbnet" => Ok(Self::Dumbnet),
            other => Err(format!("unknown settlement mode: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime settlement config (resolved from all sources)
// ---------------------------------------------------------------------------

/// Fully-resolved settlement configuration used at runtime.
#[derive(Clone)]
pub struct SettlementConfig {
    pub mode: SettlementMode,
    /// gRPC endpoint (e.g. "http://localhost:9090"). None for local mode.
    pub chain_endpoint: Option<String>,
    /// Signing key. None for local mode (and dumbnet until wallet init).
    pub signing_key: Option<[Belt; 8]>,
    pub coinbase_timelock_min: u64,
    pub tx_fee: u64,
    /// Whether to auto-submit settlement transactions on-chain.
    pub auto_submit: bool,
    /// How long to wait for TX acceptance before giving up (seconds).
    /// Fakenet: 300s, dumbnet: 900s (blocks are ~10min).
    pub accept_timeout_secs: u64,
}

impl SettlementConfig {
    /// Local mode — zero config, zero chain.
    pub fn local() -> Self {
        Self {
            mode: SettlementMode::Local,
            chain_endpoint: None,
            signing_key: None,
            coinbase_timelock_min: 1,
            tx_fee: 256,
            auto_submit: false,
            accept_timeout_secs: 0,
        }
    }

    /// Resolve config from CLI args, toml, and mode defaults.
    ///
    /// Resolution order: CLI > env > toml > mode defaults.
    /// Backward compat: `--chain-endpoint` without `--settlement-mode` infers fakenet.
    pub fn resolve(
        cli_mode: Option<SettlementMode>,
        cli_chain_endpoint: Option<String>,
        cli_submit: bool,
        cli_tx_fee: Option<u64>,
        cli_coinbase_timelock_min: Option<u64>,
        cli_accept_timeout: Option<u64>,
        cli_seed_phrase: Option<String>,
        toml: &VeslConfig,
    ) -> Self {
        // 1. Determine mode: CLI > toml > infer from flags > local
        let mode = cli_mode
            .or_else(|| {
                toml.settlement_mode
                    .as_deref()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or_else(|| {
                // Backward compat: --chain-endpoint or --submit without mode → fakenet
                if cli_chain_endpoint.is_some() || cli_submit {
                    SettlementMode::Fakenet
                } else {
                    SettlementMode::Local
                }
            });

        match mode {
            SettlementMode::Local => Self::local(),

            SettlementMode::Fakenet => Self {
                mode: SettlementMode::Fakenet,
                chain_endpoint: Some(
                    cli_chain_endpoint
                        .or_else(|| toml.chain_endpoint.clone())
                        .unwrap_or_else(|| "http://localhost:9090".into()),
                ),
                signing_key: Some(signing::demo_signing_key()),
                coinbase_timelock_min: cli_coinbase_timelock_min
                    .or(toml.coinbase_timelock_min)
                    .unwrap_or(1),
                tx_fee: cli_tx_fee.or(toml.tx_fee).unwrap_or(256),
                auto_submit: true,
                accept_timeout_secs: cli_accept_timeout
                    .or(toml.accept_timeout_secs)
                    .unwrap_or(300),
            },

            SettlementMode::Dumbnet => {
                let endpoint = cli_chain_endpoint
                    .or_else(|| toml.chain_endpoint.clone())
                    .expect(
                        "dumbnet mode requires --chain-endpoint or chain_endpoint in vesl.toml",
                    );

                // Resolve signing key: CLI seed phrase > env var > None
                let seed = cli_seed_phrase
                    .or_else(|| std::env::var("VESL_SEED_PHRASE").ok());
                let sk = seed.map(|s| signing::key_from_seed_phrase(&s)
                    .expect("invalid seed phrase — produced zero scalar"));

                Self {
                    mode: SettlementMode::Dumbnet,
                    chain_endpoint: Some(endpoint),
                    signing_key: sk,
                    coinbase_timelock_min: cli_coinbase_timelock_min
                        .or(toml.coinbase_timelock_min)
                        .unwrap_or(1),
                    tx_fee: cli_tx_fee.or(toml.tx_fee).unwrap_or(256),
                    auto_submit: true,
                    accept_timeout_secs: cli_accept_timeout
                        .or(toml.accept_timeout_secs)
                        .unwrap_or(900),
                }
            }
        }
    }

    /// True if this config has everything needed for on-chain submission.
    pub fn can_submit(&self) -> bool {
        self.auto_submit && self.chain_endpoint.is_some() && self.signing_key.is_some()
    }

    /// Build a `ChainConfig` using this settlement config's endpoint and timeout.
    /// Returns `None` for local mode (no endpoint).
    pub fn chain_config(&self) -> Option<crate::chain::ChainConfig> {
        self.chain_endpoint.as_ref().map(|ep| {
            crate::chain::ChainConfig {
                endpoint: ep.clone(),
                poll_interval: std::time::Duration::from_secs(5),
                accept_timeout: std::time::Duration::from_secs(self.accept_timeout_secs),
            }
        })
    }
}

impl fmt::Display for SettlementConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mode={}", self.mode)?;
        if let Some(ref ep) = self.chain_endpoint {
            write!(f, " endpoint={ep}")?;
        }
        if self.signing_key.is_some() {
            let label = if self.signing_key.map_or(false, |sk| signing::is_demo_key(&sk)) {
                "demo"
            } else {
                "custom"
            };
            write!(f, " key={label}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// vesl.toml config file
// ---------------------------------------------------------------------------

/// Deserializable config from `vesl.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct VeslConfig {
    pub nock_home: Option<String>,
    pub ollama_url: Option<String>,
    pub api_port: Option<u16>,
    pub settlement_mode: Option<String>,
    pub chain_endpoint: Option<String>,
    pub tx_fee: Option<u64>,
    pub coinbase_timelock_min: Option<u64>,
    pub accept_timeout_secs: Option<u64>,
}

/// Load config from a TOML file. Returns defaults if the file doesn't exist.
pub fn load_config(path: &Path) -> VeslConfig {
    match std::fs::read_to_string(path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("warn: failed to parse {}: {e}", path.display());
                VeslConfig::default()
            }
        },
        Err(_) => VeslConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_local() {
        let toml = VeslConfig::default();
        let cfg = SettlementConfig::resolve(None, None, false, None, None, None, None, &toml);
        assert_eq!(cfg.mode, SettlementMode::Local);
        assert!(cfg.chain_endpoint.is_none());
        assert!(cfg.signing_key.is_none());
        assert!(!cfg.auto_submit);
    }

    #[test]
    fn chain_endpoint_infers_fakenet() {
        let toml = VeslConfig::default();
        let cfg = SettlementConfig::resolve(
            None,
            Some("http://localhost:9090".into()),
            false,
            None,
            None,
            None,
            None,
            &toml,
        );
        assert_eq!(cfg.mode, SettlementMode::Fakenet);
        assert!(cfg.signing_key.is_some());
        assert!(cfg.auto_submit);
    }

    #[test]
    fn submit_flag_infers_fakenet() {
        let toml = VeslConfig::default();
        let cfg = SettlementConfig::resolve(None, None, true, None, None, None, None, &toml);
        assert_eq!(cfg.mode, SettlementMode::Fakenet);
    }

    #[test]
    fn explicit_local_ignores_chain_endpoint() {
        let toml = VeslConfig::default();
        let cfg = SettlementConfig::resolve(
            Some(SettlementMode::Local),
            Some("http://localhost:9090".into()),
            true,
            None,
            None,
            None,
            None,
            &toml,
        );
        assert_eq!(cfg.mode, SettlementMode::Local);
        assert!(cfg.chain_endpoint.is_none());
        assert!(!cfg.auto_submit);
    }

    #[test]
    fn fakenet_defaults() {
        let toml = VeslConfig::default();
        let cfg = SettlementConfig::resolve(
            Some(SettlementMode::Fakenet),
            None,
            false,
            None,
            None,
            None,
            None,
            &toml,
        );
        assert_eq!(cfg.chain_endpoint.as_deref(), Some("http://localhost:9090"));
        assert_eq!(cfg.tx_fee, 256);
        assert_eq!(cfg.coinbase_timelock_min, 1);
        assert_eq!(cfg.accept_timeout_secs, 300);
        assert!(cfg.signing_key.is_some());
    }

    #[test]
    fn toml_overrides_defaults() {
        let toml = VeslConfig {
            tx_fee: Some(5000),
            coinbase_timelock_min: Some(10),
            chain_endpoint: Some("http://custom:9090".into()),
            ..Default::default()
        };
        let cfg = SettlementConfig::resolve(
            Some(SettlementMode::Fakenet),
            None,
            false,
            None,
            None,
            None,
            None,
            &toml,
        );
        assert_eq!(cfg.tx_fee, 5000);
        assert_eq!(cfg.coinbase_timelock_min, 10);
        assert_eq!(cfg.chain_endpoint.as_deref(), Some("http://custom:9090"));
    }

    #[test]
    fn cli_overrides_toml() {
        let toml = VeslConfig {
            tx_fee: Some(5000),
            ..Default::default()
        };
        let cfg = SettlementConfig::resolve(
            Some(SettlementMode::Fakenet),
            Some("http://cli:9090".into()),
            false,
            Some(7000),
            None,
            None,
            None,
            &toml,
        );
        assert_eq!(cfg.tx_fee, 7000);
        assert_eq!(cfg.chain_endpoint.as_deref(), Some("http://cli:9090"));
    }

    #[test]
    fn toml_settlement_mode_parsed() {
        let toml = VeslConfig {
            settlement_mode: Some("fakenet".into()),
            ..Default::default()
        };
        let cfg = SettlementConfig::resolve(None, None, false, None, None, None, None, &toml);
        assert_eq!(cfg.mode, SettlementMode::Fakenet);
    }

    #[test]
    fn dumbnet_with_seed_phrase() {
        let toml = VeslConfig {
            chain_endpoint: Some("http://node:9090".into()),
            ..Default::default()
        };
        let cfg = SettlementConfig::resolve(
            Some(SettlementMode::Dumbnet),
            None,
            false,
            None,
            None,
            None,
            Some("test seed phrase for key derivation".into()),
            &toml,
        );
        assert_eq!(cfg.mode, SettlementMode::Dumbnet);
        assert!(cfg.signing_key.is_some());
        assert!(cfg.auto_submit);
        assert_eq!(cfg.accept_timeout_secs, 900);
        // Key should NOT be the demo key
        assert!(!signing::is_demo_key(&cfg.signing_key.unwrap()));
    }

    #[test]
    fn can_submit_checks() {
        let local = SettlementConfig::local();
        assert!(!local.can_submit());

        let fakenet = SettlementConfig::resolve(
            Some(SettlementMode::Fakenet),
            None,
            false,
            None,
            None,
            None,
            None,
            &VeslConfig::default(),
        );
        assert!(fakenet.can_submit());
    }

    #[test]
    fn settlement_mode_display_roundtrip() {
        for mode in [SettlementMode::Local, SettlementMode::Fakenet, SettlementMode::Dumbnet] {
            let s = mode.to_string();
            let parsed: SettlementMode = s.parse().unwrap();
            assert_eq!(mode, parsed);
        }
    }
}
