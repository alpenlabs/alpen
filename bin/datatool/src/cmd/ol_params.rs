//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::{fs, path::Path};

use alpen_ee_config::AlpenEeParams;
use anyhow::{anyhow, bail};
use strata_btc_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::AccountId;
use strata_ol_params::{GenesisSnarkAccountData, OLParams};
use strata_primitives::Buf32;
use strata_snark_acct_runtime::IInnerState;

use crate::{
    acct_predicate::resolve_acct_predicate,
    args::{CmdContext, SubcOlParams},
    cmd::{
        ee_params::read_chain_config,
        genesis_info::{get_alpen_ee_genesis_block_info, retrieve_l1_anchor},
    },
};

const ALPEN_EE_ACCOUNT_ID: AccountId = AccountId::new([1u8; 32]);

/// Executes the `gen-ol-params` subcommand.
///
/// Generates the OL params for a Strata network by retrieving the genesis L1
/// anchor and constructing an [`OLParams`] with a pre-registered Alpen EE snark
/// account. Outputs the result as pretty-printed JSON, either to the specified
/// file or to stdout.
///
/// The snark account's inner state root can be specified in three ways:
/// - `--ee-params`: path to EE params, from which the genesis block hash and state root are used to
///   compute the inner state root.
/// - `--alpen-chain-config`: path to the EVM chain config JSON, from which the genesis block hash
///   is extracted and used to compute the inner state root.
/// - `--alpen-inner-state`: explicit 64-char hex value, overrides chain config. If neither is
///   provided, computes from the default dev chain spec.
pub(super) fn exec(cmd: SubcOlParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    ensure_consistent_inner_state_args(cmd.ee_params.as_deref(), cmd.alpen_inner_state.as_deref())?;

    let anchor = retrieve_l1_anchor(cmd.l1_anchor_file.as_deref(), cmd.genesis_l1_height, ctx)?;

    let mut ol_params = OLParams::new_empty(anchor.block);
    let ee_params = read_ee_params(cmd.ee_params.as_deref())?;

    let acct_predicate = resolve_acct_predicate(cmd.alpen_predicate)?;
    let balance = BitcoinAmount::from_sat(cmd.alpen_balance.unwrap_or(0));
    let inner_state = match cmd.alpen_inner_state {
        Some(hex) => hex
            .parse::<Buf32>()
            .map_err(|e| anyhow!("invalid alpen-inner-state hex: {e}"))?,
        None => match &ee_params {
            Some(params) => {
                compute_inner_state(params.genesis_blockhash(), params.genesis_stateroot())
            }
            None => compute_inner_state_from_chain_config(cmd.alpen_chain_config.as_deref())?,
        },
    };
    let alpen_ee_account = GenesisSnarkAccountData {
        predicate: acct_predicate,
        inner_state,
        balance,
    };
    let account_id = ee_params
        .as_ref()
        .map(AlpenEeParams::account_id)
        .unwrap_or(ALPEN_EE_ACCOUNT_ID);
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

fn ensure_consistent_inner_state_args(
    ee_params: Option<&Path>,
    alpen_inner_state: Option<&str>,
) -> anyhow::Result<()> {
    if ee_params.is_some() && alpen_inner_state.is_some() {
        bail!("--alpen-inner-state cannot be used with --ee-params");
    }

    Ok(())
}

fn read_ee_params(path: Option<&Path>) -> anyhow::Result<Option<AlpenEeParams>> {
    let Some(path) = path else {
        return Ok(None);
    };

    let json = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed to read EE params file {path:?}: {e}"))?;
    let params = AlpenEeParams::from_json_str(&json)
        .map_err(|e| anyhow!("failed to parse EE params file {path:?}: {e}"))?;
    Ok(Some(params))
}

fn compute_inner_state_from_chain_config(chain_config: Option<&Path>) -> anyhow::Result<Buf32> {
    let genesis_json = read_chain_config(chain_config)?;
    let genesis_info = get_alpen_ee_genesis_block_info(&genesis_json)?;
    Ok(compute_inner_state(
        genesis_info.blockhash,
        genesis_info.stateroot,
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
    use std::path::Path;

    use super::ensure_consistent_inner_state_args;

    #[test]
    fn rejects_explicit_inner_state_with_ee_params() {
        let err = ensure_consistent_inner_state_args(
            Some(Path::new("ee-params.json")),
            Some("0101010101010101010101010101010101010101010101010101010101010101"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("cannot be used with --ee-params"));
    }

    #[test]
    fn accepts_single_inner_state_source() {
        ensure_consistent_inner_state_args(Some(Path::new("ee-params.json")), None)
            .expect("ee params alone should be valid");
        ensure_consistent_inner_state_args(
            None,
            Some("0101010101010101010101010101010101010101010101010101010101010101"),
        )
        .expect("explicit inner state alone should be valid");
    }
}
