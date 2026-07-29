//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::{fs, path::Path};

use alpen_chainspec::ee_genesis_block_info_from_json;
use alpen_ee_config::AlpenEeParams;
use anyhow::anyhow;
use strata_btc_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_ol_params::{GenesisSnarkAccountData, OLParams};
use strata_primitives::Buf32;
use strata_snark_acct_runtime::IInnerState;

use crate::{
    acct_predicate::resolve_acct_predicate,
    args::{CmdContext, SubcOlParams},
    cmd::{ee_params::read_chain_config, genesis_info::retrieve_l1_anchor},
};

/// Executes the `gen-ol-params` subcommand.
///
/// Generates the OL params for a Strata network by retrieving the genesis L1
/// anchor and constructing an [`OLParams`] with a pre-registered Alpen EE snark
/// account. Outputs the result as pretty-printed JSON, either to the specified
/// file or to stdout.
///
/// The snark account's inner state root comes from `--ee-params` unless explicitly overridden,
/// while bridge params always come from `--ee-params`:
/// - `--ee-params`: path to EE params, from which the account id and bridge params are copied. The
///   genesis block hash and state root are used to compute the inner state root unless
///   `--alpen-chain-config` or `--alpen-inner-state` is provided.
/// - `--alpen-chain-config`: optional path to EVM chain config JSON used to derive the inner state
///   root.
/// - `--alpen-inner-state`: explicit 64-char hex value, overrides the inner state derived from
///   `--alpen-chain-config` or EE params.
pub(super) fn exec(cmd: SubcOlParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let anchor = retrieve_l1_anchor(cmd.l1_anchor_file.as_deref(), cmd.genesis_l1_height, ctx)?;
    let ee_params_path = cmd
        .ee_params
        .as_deref()
        .ok_or_else(|| anyhow!("--ee-params is required so OL params can include bridge params"))?;
    let ee_params = read_ee_params(ee_params_path)?;
    let mut ol_params = OLParams::new_empty(anchor.block, *ee_params.bridge_params());

    let acct_predicate = resolve_acct_predicate(cmd.alpen_predicate)?;
    let balance = BitcoinAmount::from_sat(cmd.alpen_balance.unwrap_or(0));
    let inner_state = resolve_inner_state(
        cmd.alpen_inner_state.as_deref(),
        cmd.alpen_chain_config.as_deref(),
        &ee_params,
    )?;
    let alpen_ee_account = GenesisSnarkAccountData {
        predicate: acct_predicate,
        inner_state,
        balance,
    };
    let account_id = ee_params.account_id();
    ol_params.accounts.insert(account_id, alpen_ee_account);

    let params_buf = serde_json::to_string_pretty(&ol_params)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, &params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}

fn read_ee_params(path: &Path) -> anyhow::Result<AlpenEeParams> {
    let json = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed to read EE params file {path:?}: {e}"))?;
    let params = AlpenEeParams::from_json_str(&json)
        .map_err(|e| anyhow!("failed to parse EE params file {path:?}: {e}"))?;
    Ok(params)
}

fn resolve_inner_state(
    alpen_inner_state: Option<&str>,
    alpen_chain_config: Option<&Path>,
    ee_params: &AlpenEeParams,
) -> anyhow::Result<Buf32> {
    if let Some(hex) = alpen_inner_state {
        return hex
            .parse::<Buf32>()
            .map_err(|e| anyhow!("invalid alpen-inner-state hex: {e}"));
    }

    if let Some(alpen_chain_config) = alpen_chain_config {
        return compute_inner_state_from_chain_config(alpen_chain_config);
    }

    Ok(compute_inner_state(
        ee_params.genesis_blockhash(),
        ee_params.genesis_stateroot(),
    ))
}

fn compute_inner_state_from_chain_config(chain_config: &Path) -> anyhow::Result<Buf32> {
    let genesis_json = read_chain_config(Some(chain_config))?;
    let genesis_info = ee_genesis_block_info_from_json(&genesis_json)?;
    Ok(compute_inner_state(
        genesis_info.blockhash(),
        genesis_info.stateroot(),
    ))
}

fn compute_inner_state(
    blockhash: alloy_primitives::B256,
    state_root: alloy_primitives::B256,
) -> Buf32 {
    let blockhash = blockhash.0.into();
    let state_root = state_root.0.into();
    let ee_state = EeAccountState::new(blockhash, state_root, Vec::new(), Vec::new());
    ee_state.compute_state_root()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use alpen_chainspec::DEV_CHAIN_SPEC;
    use alpen_ee_config::{AlpenEeParams, DEFAULT_ALPEN_EE_ACCOUNT_ID};
    use strata_ol_params::BridgeParams;

    use super::{compute_inner_state, resolve_inner_state};

    fn test_bridge_params() -> BridgeParams {
        BridgeParams::new_with_descriptor_limit(100_000_000, Some(1_000_000_000), 81)
            .expect("valid bridge params")
    }

    fn test_ee_params(blockhash_byte: u8, stateroot_byte: u8) -> AlpenEeParams {
        AlpenEeParams::new(
            DEFAULT_ALPEN_EE_ACCOUNT_ID,
            [blockhash_byte; 32].into(),
            [stateroot_byte; 32].into(),
            0,
            test_bridge_params(),
        )
    }

    #[test]
    fn explicit_inner_state_overrides_chain_config_and_ee_params() {
        let params = test_ee_params(1, 2);
        let expected = "abababababababababababababababababababababababababababababababab"
            .parse()
            .unwrap();

        let resolved = resolve_inner_state(
            Some("abababababababababababababababababababababababababababababababab"),
            Some(Path::new("does-not-need-to-exist.json")),
            &params,
        )
        .unwrap();

        assert_eq!(resolved, expected);
    }

    #[test]
    fn chain_config_overrides_ee_params_when_inner_state_is_absent() {
        let params = test_ee_params(1, 2);
        let tempdir = tempfile::tempdir().unwrap();
        let chain_config_path = tempdir.path().join("chain.json");
        fs::write(&chain_config_path, DEV_CHAIN_SPEC).unwrap();

        let resolved = resolve_inner_state(None, Some(&chain_config_path), &params).unwrap();
        let genesis_info = alpen_chainspec::ee_genesis_block_info_from_json(DEV_CHAIN_SPEC)
            .expect("valid dev chain spec");
        let expected = compute_inner_state(genesis_info.blockhash(), genesis_info.stateroot());

        assert_eq!(resolved, expected);
    }

    #[test]
    fn ee_params_supply_inner_state_when_no_override_is_present() {
        let params = test_ee_params(1, 2);
        let resolved = resolve_inner_state(None, None, &params).unwrap();
        let expected = compute_inner_state(params.genesis_blockhash(), params.genesis_stateroot());

        assert_eq!(resolved, expected);
    }
}
