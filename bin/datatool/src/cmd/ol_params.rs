//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::fs;

use strata_btc_types::BitcoinAmount;
use strata_identifiers::AccountId;
use strata_ol_params::{GenesisSnarkAccountData, OLParams};
use strata_primitives::Buf32;

use crate::{
    acct_predicate::resolve_acct_predicate,
    args::{CmdContext, SubcOlParams},
};

const ALPEN_EE_ACCOUNT_ID: AccountId = AccountId::new([1u8; 32]);

/// Executes the `gen-ol-params` subcommand.
///
/// Generates the OL params for a Strata network by retrieving the genesis L1
/// view and constructing an [`OLParams`] with a pre-registered Alpen EE snark
/// account. Outputs the result as pretty-printed JSON, either to the specified
/// file or to stdout.
pub(super) fn exec(cmd: SubcOlParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let genesis_l1_view = super::params::retrieve_genesis_l1_view(
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
        None => Buf32::zero(),
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
