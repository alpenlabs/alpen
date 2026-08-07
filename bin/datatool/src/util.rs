//! Shared utility functions for the `datatool` binary.

use std::{
    env, fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use bitcoin::{bip32::Xpriv, Network};
use strata_predicate::PredicateKey;

/// Bitcoin network environment variable.
const BITCOIN_NETWORK_ENVVAR: &str = "BITCOIN_NETWORK";

/// The default network to use.
///
/// Right now this is [`Network::Signet`].
const DEFAULT_NETWORK: Network = Network::Signet;

/// Sequencer key environment variable.
pub(crate) const SEQKEY_ENVVAR: &str = "STRATA_SEQ_KEY";

/// Resolves a [`Network`] from a string.
///
/// Priority:
///
/// 1. Command-line argument (if provided)
/// 2. `BITCOIN_NETWORK` environment variable (if set)
/// 3. Default network (Signet)
pub(crate) fn resolve_network(arg: Option<&str>) -> anyhow::Result<Network> {
    // First, check if a command-line argument was provided
    if let Some(network_str) = arg {
        return match network_str {
            "signet" => Ok(Network::Signet),
            "regtest" => Ok(Network::Regtest),
            n => anyhow::bail!("unsupported network option: {n}"),
        };
    }

    // If no argument provided, check environment variable
    if let Ok(env_network) = env::var(BITCOIN_NETWORK_ENVVAR) {
        return match env_network.as_str() {
            "signet" => Ok(Network::Signet),
            "regtest" => Ok(Network::Regtest),
            n => anyhow::bail!("unsupported network option in {BITCOIN_NETWORK_ENVVAR}: {n}"),
        };
    }

    // Fall back to default
    Ok(DEFAULT_NETWORK)
}

/// Parses an abbreviated amount string.
///
/// User may or may not use suffixes to denote the amount.
///
/// # Possible suffixes (case sensitive)
///
/// - `K` for thousand.
/// - `M` for million.
/// - `G` for billion.
/// - `T` for trillion.
pub(crate) fn parse_abbr_amt(s: &str) -> anyhow::Result<u64> {
    // Thousand.
    if let Some(v) = s.strip_suffix("K") {
        return Ok(v.parse::<u64>()? * 1000);
    }

    // Million.
    if let Some(v) = s.strip_suffix("M") {
        return Ok(v.parse::<u64>()? * 1_000_000);
    }

    // Billion.
    if let Some(v) = s.strip_suffix("G") {
        return Ok(v.parse::<u64>()? * 1_000_000_000);
    }

    // Trillion, probably not necessary.
    if let Some(v) = s.strip_suffix("T") {
        return Ok(v.parse::<u64>()? * 1_000_000_000_000);
    }

    // Simple value.
    Ok(s.parse::<u64>()?)
}

/// Reads a serialized [`PredicateKey`] from a metadata file.
pub(crate) fn read_predicate_key(path: &Path, label: &str) -> anyhow::Result<PredicateKey> {
    let serialized = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {label} predicate file {path:?}: {e}"))?;
    let serialized = serialized.trim();
    let predicate: PredicateKey =
        serde_json::from_value(serde_json::Value::String(serialized.to_owned()))
            .map_err(|e| anyhow::anyhow!("failed to parse {label} predicate file {path:?}: {e}"))?;
    Ok(predicate)
}

/// Resolves an [`Xpriv`] from the file path (if provided) or environment variable (if
/// `--key-from-env` set). Only one source should be specified.
///
/// Priority:
///
/// 1. File path (if provided with path argument)
/// 2. Environment variable (if --key-from-env flag is set)
pub(crate) fn resolve_xpriv(
    path: &Option<PathBuf>,
    from_env: bool,
    env: &'static str,
) -> anyhow::Result<Option<Xpriv>> {
    match (path, from_env) {
        (Some(_), true) => anyhow::bail!("got key path and --key-from-env, pick a lane"),
        (Some(path), false) => Ok(Some(read_xpriv(path)?)),
        (None, true) => parse_xpriv_from_env(env).map(Some),
        _ => Ok(None),
    }
}

/// Reads an [`Xpriv`] from file as a string and verifies the checksum.
fn read_xpriv(path: &Path) -> anyhow::Result<Xpriv> {
    let xpriv = Xpriv::from_str(&fs::read_to_string(path)?)?;
    Ok(xpriv)
}

/// Parses an [`Xpriv`] from environment variable.
fn parse_xpriv_from_env(env: &'static str) -> anyhow::Result<Xpriv> {
    let env_val = match env::var(env) {
        Ok(v) => v,
        Err(_) => anyhow::bail!("got --key-from-env but {env} not set or invalid"),
    };

    let xpriv = match Xpriv::from_str(&env_val) {
        Ok(xpriv) => xpriv,
        Err(_) => anyhow::bail!("got --key-from-env but invalid xpriv"),
    };

    Ok(xpriv)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use strata_predicate::{PredicateKey, PredicateTypeId};

    use super::read_predicate_key;

    #[test]
    fn reads_predicate_key_from_metadata_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "AlwaysAccept:\n").unwrap();

        let predicate = read_predicate_key(&path, "test").unwrap();

        assert_eq!(predicate, PredicateKey::always_accept());
    }

    #[test]
    fn reads_always_accept_shorthand() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "AlwaysAccept\n").unwrap();

        let predicate = read_predicate_key(&path, "test").unwrap();

        assert_eq!(predicate, PredicateKey::always_accept());
    }

    #[test]
    fn rejects_invalid_predicate_metadata_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "not-a-predicate\n").unwrap();

        let err = read_predicate_key(&path, "test").unwrap_err();

        assert!(err.to_string().contains("failed to parse test predicate"));
    }

    #[test]
    fn preserves_predicate_condition_bytes() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "Bip340Schnorr:010203\n").unwrap();

        let predicate = read_predicate_key(&path, "test").unwrap();

        assert_eq!(predicate.id(), PredicateTypeId::Bip340Schnorr.as_u8());
        assert_eq!(predicate.condition(), &[1, 2, 3]);
    }
}
