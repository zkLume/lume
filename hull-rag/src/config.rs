//! Settlement mode configuration — hull-rag layer.
//!
//! Re-exports generic config from vesl-core and adds:
//! - VeslConfig (domain-specific toml fields like ollama_url)
//! - clap::ValueEnum for SettlementMode
//! - load_config() for reading vesl.toml

use std::path::Path;

use serde::Deserialize;

pub use vesl_core::config::{
    SettlementConfig, SettlementMode, SettlementToml,
};

use crate::signing;

// ---------------------------------------------------------------------------
// VeslConfig — domain-specific toml fields
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

impl From<&VeslConfig> for SettlementToml {
    fn from(v: &VeslConfig) -> Self {
        Self {
            settlement_mode: v.settlement_mode.clone(),
            chain_endpoint: v.chain_endpoint.clone(),
            tx_fee: v.tx_fee,
            coinbase_timelock_min: v.coinbase_timelock_min,
            accept_timeout_secs: v.accept_timeout_secs,
        }
    }
}

/// Load config from a TOML file. Returns defaults if the file doesn't exist.
pub fn load_config(path: &Path) -> VeslConfig {
    match std::fs::read_to_string(path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("WARNING: failed to parse {}: {e} — using default config", path.display());
                VeslConfig::default()
            }
        },
        Err(_) => VeslConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Convenience: resolve with demo key for fakenet
// ---------------------------------------------------------------------------

/// Resolve settlement config with hull-rag defaults (demo key for fakenet).
///
/// Thin wrapper around `SettlementConfig::resolve()` that passes the
/// demo signing key as `default_signing_key`. Preserved for tests.
pub fn resolve_with_demo_key(
    cli_mode: Option<SettlementMode>,
    cli_chain_endpoint: Option<String>,
    cli_submit: bool,
    cli_tx_fee: Option<u64>,
    cli_coinbase_timelock_min: Option<u64>,
    cli_accept_timeout: Option<u64>,
    cli_seed_phrase: Option<String>,
    toml: &VeslConfig,
) -> SettlementConfig {
    let settlement_toml = SettlementToml::from(toml);
    SettlementConfig::resolve(
        cli_mode,
        cli_chain_endpoint,
        cli_submit,
        cli_tx_fee,
        cli_coinbase_timelock_min,
        cli_accept_timeout,
        cli_seed_phrase,
        &settlement_toml,
        Some(signing::demo_signing_key()),
    )
}

/// Checked variant — surfaces misconfiguration as a typed error for main.rs (L-14).
pub fn resolve_with_demo_key_checked(
    cli_mode: Option<SettlementMode>,
    cli_chain_endpoint: Option<String>,
    cli_submit: bool,
    cli_tx_fee: Option<u64>,
    cli_coinbase_timelock_min: Option<u64>,
    cli_accept_timeout: Option<u64>,
    cli_seed_phrase: Option<String>,
    toml: &VeslConfig,
) -> Result<SettlementConfig, String> {
    let settlement_toml = SettlementToml::from(toml);
    SettlementConfig::resolve_checked(
        cli_mode,
        cli_chain_endpoint,
        cli_submit,
        cli_tx_fee,
        cli_coinbase_timelock_min,
        cli_accept_timeout,
        cli_seed_phrase,
        &settlement_toml,
        Some(signing::demo_signing_key()),
    )
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
        let cfg = resolve_with_demo_key(None, None, false, None, None, None, None, &toml);
        assert_eq!(cfg.mode, SettlementMode::Local);
        assert!(cfg.chain_endpoint.is_none());
        assert!(cfg.signing_key.is_none());
        assert!(!cfg.auto_submit);
    }

    #[test]
    fn chain_endpoint_infers_fakenet() {
        let toml = VeslConfig::default();
        let cfg = resolve_with_demo_key(
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
        let cfg = resolve_with_demo_key(None, None, true, None, None, None, None, &toml);
        assert_eq!(cfg.mode, SettlementMode::Fakenet);
    }

    #[test]
    fn explicit_local_ignores_chain_endpoint() {
        let toml = VeslConfig::default();
        let cfg = resolve_with_demo_key(
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
        let cfg = resolve_with_demo_key(
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
        let cfg = resolve_with_demo_key(
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
        let cfg = resolve_with_demo_key(
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
        let cfg = resolve_with_demo_key(None, None, false, None, None, None, None, &toml);
        assert_eq!(cfg.mode, SettlementMode::Fakenet);
    }

    #[test]
    fn dumbnet_with_seed_phrase() {
        let toml = VeslConfig {
            chain_endpoint: Some("http://node:9090".into()),
            ..Default::default()
        };
        let cfg = resolve_with_demo_key(
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
        assert!(!signing::is_demo_key(&cfg.signing_key.unwrap()));
    }

    #[test]
    fn can_submit_checks() {
        let local = SettlementConfig::local();
        assert!(!local.can_submit());

        let toml = VeslConfig::default();
        let fakenet = resolve_with_demo_key(
            Some(SettlementMode::Fakenet),
            None,
            false,
            None,
            None,
            None,
            None,
            &toml,
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
