//! Parses the operator's master xpriv from a file.

use std::{
    env,
    fs::read_to_string,
    path::{Path, PathBuf},
};

use bitcoin::bip32::Xpriv;
use strata_key_derivation::operator::OperatorKeys;
use strata_primitives::keys::ZeroizableXpriv;
use zeroize::Zeroize;

/// The environment variable that contains the operator's master [`Xpriv`].
const OPXPRIV_ENVVAR: &str = "STRATA_OP_MASTER_XPRIV";

/// Parses the master [`Xpriv`] from a file.
pub(crate) fn parse_master_xpriv(path: &Path) -> anyhow::Result<OperatorKeys> {
    let mut xpriv_str = read_to_string(path)?;
    match xpriv_str.parse::<Xpriv>() {
        Ok(mut xpriv) => {
            // Zeroize the xpriv string after parsing it.
            xpriv_str.zeroize();

            // Parse into ZeroizableXpriv
            let zeroizable_xpriv: ZeroizableXpriv = xpriv.into();

            // Zeroize the xpriv after parsing it.
            xpriv.private_key.non_secure_erase();

            // Finally return the operator keys
            //
            // NOTE: `zeroizable_xpriv` is zeroized on drop.
            Ok(OperatorKeys::new(&zeroizable_xpriv)
                .map_err(|_| anyhow::anyhow!("invalid master xpriv"))?)
        }
        Err(e) => anyhow::bail!("invalid master xpriv: {}", e),
    }
}

/// Resolves the master [`Xpriv`] from CLI arguments or environment variables.
///
/// Precedence order for resolving the master xpriv:
///
/// 1. If a key is supplied via the `--master-xpriv` CLI argument, it is used.
/// 2. Otherwise, if a file path is supplied via CLI, the key is read from that file.
/// 3. Otherwise, if the `STRATA_OP_MASTER_XPRIV` environment variable is set, its value is used.
/// 4. Otherwise, returns an error.
///
/// # Errors
///
/// Returns an error if the master xpriv is invalid or not found.
pub(crate) fn resolve_xpriv(
    cli_arg: Option<String>,
    cli_path: Option<String>,
) -> anyhow::Result<OperatorKeys> {
    match (cli_arg, cli_path) {
        (Some(xpriv), _) => OperatorKeys::new(&xpriv.parse::<Xpriv>()?)
            .map_err(|_| anyhow::anyhow!("invalid master xpriv from CLI")),

        (_, Some(path)) => parse_master_xpriv(&PathBuf::from(path)),

        (None, None) => match env::var(OPXPRIV_ENVVAR) {
            Ok(xpriv_env_str) => OperatorKeys::new(&xpriv_env_str.parse::<Xpriv>()?)
                .map_err(|_| anyhow::anyhow!("invalid master xpriv from envvar")),
            Err(_) => {
                anyhow::bail!(
                    "must either set {OPXPRIV_ENVVAR} envvar or pass with `--master-xpriv`"
                )
            }
        },
    }
}
