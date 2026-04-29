//! Verifier configuration.

use std::{fs, path::Path};

use serde::{de::Error as DeError, Deserialize, Deserializer};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_l1_txfmt::MagicBytes;

/// Deserializes a `MagicBytes` from a TOML string via its `FromStr` impl
/// (4 ASCII bytes).
fn deserialize_magic_bytes<'de, D>(deserializer: D) -> Result<MagicBytes, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    value.parse().map_err(DeError::custom)
}

/// Effective verifier configuration loaded from the config file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct VerifierConfig {
    pub(crate) bitcoind_url: String,
    pub(crate) bitcoind_rpc_user: String,
    pub(crate) bitcoind_rpc_password: String,
    #[serde(deserialize_with = "deserialize_magic_bytes")]
    pub(crate) magic_bytes: MagicBytes,
    pub(crate) chain_spec: String,
}

impl VerifierConfig {
    /// Loads and parses verifier configuration from a TOML file.
    pub(crate) fn load(path: &Path) -> Result<Self, DisplayedError> {
        let contents =
            fs::read_to_string(path).user_error(format!("failed to read {}", path.display()))?;
        Self::parse_toml(path, &contents)
    }

    /// Parses verifier configuration from TOML contents.
    fn parse_toml(path: &Path, contents: &str) -> Result<Self, DisplayedError> {
        toml::from_str::<Self>(contents).user_error(format!("failed to parse {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use strata_l1_txfmt::MagicBytes;

    use super::VerifierConfig;

    fn valid_config_toml() -> &'static str {
        r#"
bitcoind_url = "http://127.0.0.1:18443"
bitcoind_rpc_user = "rpc_user"
bitcoind_rpc_password = "rpc_password"
magic_bytes = "STRA"
chain_spec = "dev"
"#
    }

    fn test_path() -> &'static Path {
        Path::new("test.toml")
    }

    #[test]
    fn parse_toml_succeeds_for_valid_config() {
        let config = VerifierConfig::parse_toml(test_path(), valid_config_toml())
            .expect("config must parse");
        assert_eq!(config.bitcoind_url, "http://127.0.0.1:18443");
        assert_eq!(config.bitcoind_rpc_user, "rpc_user");
        assert_eq!(config.bitcoind_rpc_password, "rpc_password");
        assert_eq!(config.magic_bytes, MagicBytes::new(*b"STRA"));
        assert_eq!(config.chain_spec, "dev");
    }

    #[test]
    fn parse_toml_fails_when_required_field_missing() {
        VerifierConfig::parse_toml(
            test_path(),
            r#"
bitcoind_url = "http://127.0.0.1:18443"
bitcoind_rpc_user = "rpc_user"
bitcoind_rpc_password = "rpc_password"
magic_bytes = "STRA"
"#,
        )
        .expect_err("missing field must fail");
    }

    #[test]
    fn load_fails_for_nonexistent_file() {
        VerifierConfig::load(Path::new("/nonexistent/config.toml"))
            .expect_err("missing file must fail");
    }

    #[test]
    fn parse_toml_fails_when_magic_bytes_invalid() {
        VerifierConfig::parse_toml(
            test_path(),
            r#"
bitcoind_url = "http://127.0.0.1:18443"
bitcoind_rpc_user = "rpc_user"
bitcoind_rpc_password = "rpc_password"
magic_bytes = "XYZ"
chain_spec = "dev"
"#,
        )
        .expect_err("invalid magic must fail");
    }
}
