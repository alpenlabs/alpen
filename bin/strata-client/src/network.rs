//! Hardcoded network configuration and resolution.

use std::{
    env::{self, VarError},
    fs,
};

use strata_params::RollupParams;
use tracing::warn;

/// Rollup params we initialize with if not overridden.  Optionally set at compile time.
pub(crate) const DEFAULT_NETWORK_ROLLUP_PARAMS: Option<&str> = option_env!("STRATA_NETWORK_PARAMS");

/// Envvar we can load params from at run time.
pub(crate) const NETWORK_PARAMS_ENVVAR: &str = "STRATA_NETWORK_PARAMS";

/// Parses the default network rollup params from the hardcoded string.  Does
/// not validate them, but caller should.
pub(crate) fn get_default_rollup_params() -> anyhow::Result<RollupParams> {
    if let Some(s) = DEFAULT_NETWORK_ROLLUP_PARAMS {
        Ok(serde_json::from_str(s)?)
    } else {
        anyhow::bail!("No default network rollup parameters available. Set STRATA_NETWORK_PARAMS environment variable.")
    }
}

/// Loads the network params from the envvar, if set.  If the envvar starts with
/// `@`, then we load the file at the following path and use that instead.
pub(crate) fn get_envvar_params() -> anyhow::Result<Option<RollupParams>> {
    match env::var(NETWORK_PARAMS_ENVVAR) {
        Ok(v) => {
            let buf = if let Some(path) = v.strip_prefix("@") {
                fs::read(path)?
            } else {
                v.into_bytes()
            };
            Ok(Some(serde_json::from_slice(&buf)?))
        }
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => {
            warn!(
                "params var {} set but not UTF-8, ignoring",
                NETWORK_PARAMS_ENVVAR
            );
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_params::RollupParams;

    use super::DEFAULT_NETWORK_ROLLUP_PARAMS;

    #[test]
    fn test_params_well_formed() {
        if let Some(params_str) = DEFAULT_NETWORK_ROLLUP_PARAMS {
            let params: RollupParams =
                serde_json::from_str(params_str).expect("test: parse network params");
            params
                .check_well_formed()
                .expect("test: check network params");
        }
    }
}
