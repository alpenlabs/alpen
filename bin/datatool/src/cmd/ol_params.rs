//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::{fs, path::Path};

use alpen_chainspec::DEV_CHAIN_SPEC;
use strata_btc_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::AccountId;
use strata_ol_params::{GenesisSnarkAccountData, OLParams};
use strata_primitives::Buf32;
use strata_snark_acct_runtime::IInnerState;

use crate::{
    acct_predicate::resolve_acct_predicate,
    args::{CmdContext, SubcOlParams},
    cmd::params::{get_alpen_ee_genesis_block_info, retrieve_genesis_l1_view},
};

const ALPEN_EE_ACCOUNT_ID: AccountId = AccountId::new([1u8; 32]);

/// Executes the `gen-ol-params` subcommand.
///
/// Generates the OL params for a Strata network by retrieving the genesis L1
/// view and constructing an [`OLParams`] with a pre-registered Alpen EE snark
/// account. Outputs the result as pretty-printed JSON, either to the specified
/// file or to stdout.
///
/// The snark account's inner state root can be specified in two ways:
/// - `--alpen-chain-config`: path to the EVM chain config JSON, from which the genesis block hash
///   is extracted and used to compute the inner state root.
/// - `--alpen-inner-state`: explicit 64-char hex value, overrides chain config. If neither is
///   provided, computes from the default dev chain spec.
pub(super) fn exec(cmd: SubcOlParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let genesis_l1_view = retrieve_genesis_l1_view(
        cmd.genesis_l1_view_file.as_deref(),
        cmd.genesis_l1_height,
        ctx,
    )?;

    let mut ol_params = OLParams::new_empty(genesis_l1_view.blk);

    let acct_predicate = resolve_acct_predicate(cmd.alpen_predicate)?;
    let balance = BitcoinAmount::from_sat(cmd.alpen_balance.unwrap_or(0));
    let inner_state = match cmd.alpen_inner_state {
        Some(hex) => hex
            .parse::<Buf32>()
            .map_err(|e| anyhow::anyhow!("invalid alpen-inner-state hex: {e}"))?,
        None => compute_inner_state_from_chain_config(cmd.alpen_chain_config.as_deref())?,
    };
    let alpen_ee_account = GenesisSnarkAccountData {
        predicate: acct_predicate,
        inner_state,
        balance,
    };
    ol_params
        .accounts
        .insert(ALPEN_EE_ACCOUNT_ID, alpen_ee_account);

    let params_buf = serde_json::to_string_pretty(&ol_params)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, &params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}

const DEFAULT_CHAIN_SPEC: &str = DEV_CHAIN_SPEC;

fn compute_inner_state_from_chain_config(chain_config: Option<&Path>) -> anyhow::Result<Buf32> {
    let genesis_json = match chain_config {
        Some(p) => fs::read_to_string(p)?,
        None => DEFAULT_CHAIN_SPEC.into(),
    };
    let genesis_info = get_alpen_ee_genesis_block_info(&genesis_json)?;
    let blockhash = genesis_info.blockhash.0.into();
    let ee_state = EeAccountState::new(blockhash, BitcoinAmount::zero(), Vec::new(), Vec::new());
    Ok(ee_state.compute_state_root())
}
