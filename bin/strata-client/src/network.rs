//! Hardcoded network configuration and resolution.

use std::{
    env::{self, VarError},
    fs,
};

use strata_primitives::{
    block_credential::CredRule,
    operator::OperatorPubkeys,
    params::{OperatorConfig, RollupParams},
    prelude::*,
    proof::RollupVerifyingKey,
};
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
        // TODO remove
        Ok(get_deprecated_fallback())
    }
}

/// Deprecated fallback we can load params from if not set any other way.
fn get_deprecated_fallback() -> RollupParams {
    warn!("using deprecated fallback rollup params, should only be for testing!");

    // FIXME this is broken, where are the keys?
    let opkeys = OperatorPubkeys::new(Buf32::zero(), Buf32::zero());

    // TODO: load default params from a json during compile time
    RollupParams {
        rollup_name: "strata".to_string(),
        da_tag: "strata-da".to_string(),
        checkpoint_tag: "strata-ckpt".to_string(),
        block_time: 1000,
        cred_rule: CredRule::Unchecked,
        horizon_l1_height: 3,
        genesis_l1_height: 5,
        operator_config: OperatorConfig::Static(vec![opkeys]),
        evm_genesis_block_hash:
            "0x37ad61cff1367467a98cf7c54c4ac99e989f1fbb1bc1e646235e90c065c565ba"
                .parse()
                .unwrap(),
        evm_genesis_block_state_root:
            "0x351714af72d74259f45cd7eab0b04527cd40e74836a45abcae50f92d919d988f"
                .parse()
                .unwrap(),
        l1_reorg_safe_depth: 4,
        target_l2_batch_size: 64,
        address_length: 20,
        deposit_amount: 1_000_000_000,
        rollup_vk: RollupVerifyingKey::SP1VerifyingKey(
            "0x00b01ae596b4e51843484ff71ccbd0dd1a030af70b255e6b9aad50b81d81266f"
                .parse()
                .unwrap(),
        ), // TODO: update this with vk for checkpoint proof
        dispatch_assignment_dur: 64,
        proof_publish_mode: ProofPublishMode::Timeout(5),
        max_deposits_in_block: 16,
        network: bitcoin::Network::Regtest,
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
    use strata_primitives::params::RollupParams;

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
