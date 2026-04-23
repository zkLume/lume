//! Settlement mode configuration — hull layer.
//!
//! Re-exports generic config from vesl-core and adds:
//! - HullConfig (domain-specific toml fields)
//! - load_config() for reading hull.toml / vesl.toml
//! - resolve_with_demo_key() convenience wrapper

use std::path::Path;

use serde::Deserialize;

pub use vesl_core::config::{
    SettlementConfig, SettlementMode, SettlementToml,
};

use crate::signing;

// ---------------------------------------------------------------------------
// HullConfig — toml fields for the generic hull
// ---------------------------------------------------------------------------

/// Deserializable config from `vesl.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct HullConfig {
    pub nock_home: Option<String>,
    pub api_port: Option<u16>,
    pub settlement_mode: Option<String>,
    pub chain_endpoint: Option<String>,
    pub tx_fee: Option<u64>,
    pub coinbase_timelock_min: Option<u64>,
    pub accept_timeout_secs: Option<u64>,
}

impl From<&HullConfig> for SettlementToml {
    fn from(v: &HullConfig) -> Self {
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
pub fn load_config(path: &Path) -> HullConfig {
    match std::fs::read_to_string(path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("WARNING: failed to parse {}: {e} -- using default config", path.display());
                HullConfig::default()
            }
        },
        Err(_) => HullConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Convenience: resolve with demo key for fakenet
// ---------------------------------------------------------------------------

/// Resolve settlement config with hull defaults (demo key for fakenet).
/// Preserved for tests. Production callers should prefer the checked variant.
pub fn resolve_with_demo_key(
    cli_mode: Option<SettlementMode>,
    cli_chain_endpoint: Option<String>,
    cli_submit: bool,
    cli_tx_fee: Option<u64>,
    cli_coinbase_timelock_min: Option<u64>,
    cli_accept_timeout: Option<u64>,
    cli_seed_phrase: Option<String>,
    toml: &HullConfig,
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
    toml: &HullConfig,
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
        let toml = HullConfig::default();
        let cfg = resolve_with_demo_key(None, None, false, None, None, None, None, &toml);
        assert_eq!(cfg.mode, SettlementMode::Local);
        assert!(cfg.chain_endpoint.is_none());
        assert!(cfg.signing_key.is_none());
        assert!(!cfg.auto_submit);
    }

    #[test]
    fn chain_endpoint_infers_fakenet() {
        let toml = HullConfig::default();
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
    fn explicit_local_ignores_chain_endpoint() {
        let toml = HullConfig::default();
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
        let toml = HullConfig::default();
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
    fn cli_overrides_toml() {
        let toml = HullConfig {
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
    fn settlement_mode_display_roundtrip() {
        for mode in [SettlementMode::Local, SettlementMode::Fakenet, SettlementMode::Dumbnet] {
            let s = mode.to_string();
            let parsed: SettlementMode = s.parse().unwrap();
            assert_eq!(mode, parsed);
        }
    }
}
