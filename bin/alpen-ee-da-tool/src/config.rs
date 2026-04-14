//! Verifier configuration.

use std::{error::Error, fmt, fs, io, path::Path, str::FromStr};

use bitcoin::secp256k1::XOnlyPublicKey;
use serde::{de::Error as DeError, Deserialize, Deserializer};
use strata_l1_txfmt::MagicBytes;
use terrors::OneOf;
use toml::de::Error as TomlError;

/// Deserializes a `MagicBytes` from a TOML string via its `FromStr` impl
/// (4 ASCII bytes).
fn deserialize_magic_bytes<'de, D>(deserializer: D) -> Result<MagicBytes, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    value.parse().map_err(DeError::custom)
}

/// Deserializes a 32-byte x-only public key from hex.
fn deserialize_xonly_public_key<'de, D>(deserializer: D) -> Result<XOnlyPublicKey, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    XOnlyPublicKey::from_str(&value).map_err(DeError::custom)
}

/// Effective verifier configuration loaded from the config file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct VerifierConfig {
    pub(crate) bitcoind_url: String,

    pub(crate) bitcoind_rpc_user: String,

    pub(crate) bitcoind_rpc_password: String,

    #[serde(deserialize_with = "deserialize_magic_bytes")]
    pub(crate) magic_bytes: MagicBytes,

    #[serde(deserialize_with = "deserialize_xonly_public_key")]
    pub(crate) sequencer_pubkey: XOnlyPublicKey,

    pub(crate) chain_spec: String,
}

/// Invalid verifier config TOML or invalid typed field value.
pub(crate) struct ConfigError(TomlError);

impl fmt::Debug for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl VerifierConfig {
    /// Loads and parses verifier configuration from a TOML file.
    pub(crate) fn load(path: &Path) -> Result<Self, OneOf<(io::Error, ConfigError)>> {
        let contents = fs::read_to_string(path).map_err(OneOf::new)?;
        Self::parse_toml(&contents).map_err(OneOf::new)
    }

    /// Parses verifier configuration from TOML contents.
    fn parse_toml(contents: &str) -> Result<Self, ConfigError> {
        toml::from_str::<Self>(contents).map_err(ConfigError)
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, str::FromStr};

    use bitcoin::secp256k1::XOnlyPublicKey;
    use strata_l1_txfmt::MagicBytes;

    use super::VerifierConfig;

    const TEST_SEQUENCER_PUBKEY: &str =
        "1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f";

    fn valid_config_toml() -> &'static str {
        r#"
bitcoind_url = "http://127.0.0.1:18443"
bitcoind_rpc_user = "rpc_user"
bitcoind_rpc_password = "rpc_password"
magic_bytes = "ALPN"
sequencer_pubkey = "1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f"
chain_spec = "dev"
"#
    }

    #[test]
    fn parse_toml_succeeds_for_valid_config() {
        let config = VerifierConfig::parse_toml(valid_config_toml()).expect("config must parse");
        assert_eq!(config.bitcoind_url, "http://127.0.0.1:18443");
        assert_eq!(config.bitcoind_rpc_user, "rpc_user");
        assert_eq!(config.bitcoind_rpc_password, "rpc_password");
        assert_eq!(config.magic_bytes, MagicBytes::new(*b"ALPN"));
        assert_eq!(
            config.sequencer_pubkey,
            XOnlyPublicKey::from_str(TEST_SEQUENCER_PUBKEY).expect("valid test key")
        );
        assert_eq!(config.chain_spec, "dev");
    }

    #[test]
    fn parse_toml_fails_when_required_field_missing() {
        let err = VerifierConfig::parse_toml(
            r#"
bitcoind_url = "http://127.0.0.1:18443"
bitcoind_rpc_user = "rpc_user"
bitcoind_rpc_password = "rpc_password"
magic_bytes = "ALPN"
sequencer_pubkey = "1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f"
"#,
        )
        .expect_err("missing field must fail");
        let details = format!("{err:?}");
        assert!(details.contains("missing field"));
        assert!(details.contains("chain_spec"));
    }

    #[test]
    fn load_fails_for_nonexistent_file() {
        VerifierConfig::load(Path::new("/nonexistent/config.toml"))
            .expect_err("missing file must fail");
    }

    #[test]
    fn parse_toml_fails_when_magic_bytes_invalid() {
        VerifierConfig::parse_toml(
            r#"
bitcoind_url = "http://127.0.0.1:18443"
bitcoind_rpc_user = "rpc_user"
bitcoind_rpc_password = "rpc_password"
magic_bytes = "XYZ"
sequencer_pubkey = "1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f"
chain_spec = "dev"
"#,
        )
        .expect_err("invalid magic must fail");
    }

    #[test]
    fn parse_toml_fails_when_sequencer_pubkey_invalid() {
        VerifierConfig::parse_toml(
            r#"
bitcoind_url = "http://127.0.0.1:18443"
bitcoind_rpc_user = "rpc_user"
bitcoind_rpc_password = "rpc_password"
magic_bytes = "ALPN"
sequencer_pubkey = "not-a-pubkey"
chain_spec = "dev"
"#,
        )
        .expect_err("invalid sequencer pubkey must fail");
    }
}
