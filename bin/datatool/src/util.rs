//! Shared utility functions for the `datatool` binary.

use std::{
    env, fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use bitcoin::{bip32::Xpriv, Amount, Network};
use strata_btc_types::BitcoinAmount;
use strata_predicate::PredicateKey;

/// Bitcoin network environment variable.
const BITCOIN_NETWORK_ENVVAR: &str = "BITCOIN_NETWORK";

/// The default network to use.
///
/// Right now this is [`Network::Signet`].
const DEFAULT_NETWORK: Network = Network::Signet;

/// Sequencer key environment variable.
pub(crate) const SEQKEY_ENVVAR: &str = "STRATA_SEQ_KEY";

/// Predicate metadata file label used in diagnostics.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PredicateFileLabel {
    /// ASM checkpoint predicate metadata.
    Checkpoint,
    /// Test-only predicate metadata.
    #[cfg(test)]
    Test,
}

impl PredicateFileLabel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Checkpoint => "checkpoint",
            #[cfg(test)]
            Self::Test => "test",
        }
    }
}

/// Resolves a [`Network`] from a string accepted by [`Network::from_str`].
///
/// Priority:
///
/// 1. Command-line argument (if provided)
/// 2. `BITCOIN_NETWORK` environment variable (if set)
/// 3. Default network (Signet)
pub(crate) fn resolve_network(arg: Option<Network>) -> anyhow::Result<Network> {
    // First, check if a command-line argument was provided
    if let Some(network) = arg {
        return Ok(network);
    }

    // If no argument provided, check environment variable
    if let Ok(env_network) = env::var(BITCOIN_NETWORK_ENVVAR) {
        return Network::from_str(&env_network).map_err(|_| {
            anyhow::anyhow!("unsupported network option in {BITCOIN_NETWORK_ENVVAR}: {env_network}")
        });
    }

    // Fall back to default
    Ok(DEFAULT_NETWORK)
}

/// Parses an amount given on the command line.
///
/// Accepts what [`BitcoinAmount`] accepts in a params file: a sat count such as `200000000`, or
/// a denominated amount such as `"2 BTC"`.
pub(crate) fn parse_amount(s: &str) -> Result<BitcoinAmount, String> {
    let amount = match s.parse::<u64>() {
        Ok(sats) => Amount::from_sat(sats),
        Err(_) => Amount::from_str(s).map_err(|e| e.to_string())?,
    };
    BitcoinAmount::try_from(amount.to_sat()).map_err(|e| e.to_string())
}

/// Reads a serialized [`PredicateKey`] from a metadata file.
pub(crate) fn read_predicate_key(
    path: &Path,
    label: PredicateFileLabel,
) -> anyhow::Result<PredicateKey> {
    let label = label.as_str();
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

    use super::{parse_amount, read_predicate_key, PredicateFileLabel};

    #[test]
    fn reads_predicate_key_from_metadata_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "AlwaysAccept:\n").unwrap();

        let predicate = read_predicate_key(&path, PredicateFileLabel::Test).unwrap();

        assert_eq!(predicate, PredicateKey::always_accept());
    }

    #[test]
    fn reads_always_accept_shorthand() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "AlwaysAccept\n").unwrap();

        let predicate = read_predicate_key(&path, PredicateFileLabel::Test).unwrap();

        assert_eq!(predicate, PredicateKey::always_accept());
    }

    #[test]
    fn rejects_invalid_predicate_metadata_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "not-a-predicate\n").unwrap();

        let err = read_predicate_key(&path, PredicateFileLabel::Test).unwrap_err();

        assert!(err.to_string().contains("failed to parse test predicate"));
    }

    #[test]
    fn preserves_predicate_condition_bytes() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("guest.predicate");
        fs::write(&path, "Bip340Schnorr:010203\n").unwrap();

        let predicate = read_predicate_key(&path, PredicateFileLabel::Test).unwrap();

        assert_eq!(predicate.id(), PredicateTypeId::Bip340Schnorr.as_u8());
        assert_eq!(predicate.condition(), &[1, 2, 3]);
    }

    #[test]
    fn parse_amount_accepts_sat_count_and_denominated_amount() {
        assert_eq!(parse_amount("200000000").unwrap().to_sat(), 200_000_000);
        assert_eq!(parse_amount("2 BTC").unwrap().to_sat(), 200_000_000);
        assert_eq!(parse_amount("1500 sat").unwrap().to_sat(), 1_500);
    }

    #[test]
    fn parse_amount_rejects_malformed_and_oversized_amounts() {
        assert!(parse_amount("2 coins").is_err());
        assert!(parse_amount("100M").is_err());
        assert!(parse_amount("2100000000000001").is_err());
        assert!(parse_amount("21000001 BTC").is_err());
    }
}
