//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::fs;

use strata_btc_types::BitcoinAmount;
use strata_identifiers::AccountId;
use strata_ol_params::{GenesisSnarkAccountData, OLParams};
use strata_predicate::PredicateKey;
use strata_primitives::Buf32;

use crate::args::{CmdContext, SubcOlParams};

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

    // TODO: handle EE accounts properly https://alpenlabs.atlassian.net/browse/STR-2367
    let mut ol_params = OLParams::new_empty(genesis_l1_view.blk);

    let alpen_ee_account = GenesisSnarkAccountData {
        predicate: PredicateKey::always_accept(),
        inner_state: Buf32::zero(),
        balance: BitcoinAmount::ZERO,
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
