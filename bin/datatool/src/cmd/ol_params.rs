//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::{fs, path::Path};

use alpen_ee_params::AlpenParams;
use anyhow::anyhow;
use strata_btc_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_ol_params::{GenesisSnarkAccountData, OLParams};
use strata_primitives::Buf32;
use strata_snark_acct_runtime::IInnerState;

use crate::{
    acct_predicate::resolve_acct_predicate,
    args::{CmdContext, SubcOlParams},
    cmd::genesis_info::retrieve_l1_anchor,
};

/// Executes the `gen-ol-params` subcommand.
///
/// Generates the OL params for a Strata network by retrieving the genesis L1
/// anchor and constructing an [`OLParams`] with a pre-registered Alpen EE snark
/// account. Outputs the result as pretty-printed JSON, either to the specified
/// file or to stdout.
///
/// The snark account's inner state root comes from `--alpen-params` unless
/// explicitly overridden, while bridge params always come from `--alpen-params`:
/// - `--alpen-params`: path to the Alpen params artifact, from which the account id and bridge
///   params are copied. The execution genesis block hash and state root derived from its embedded
///   EVM spec are used to compute the inner state root unless `--alpen-inner-state` is provided.
/// - `--alpen-inner-state`: explicit 64-char hex value, overrides the inner state derived from the
///   Alpen params.
pub(super) fn exec(cmd: SubcOlParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let anchor = retrieve_l1_anchor(cmd.l1_anchor_file.as_deref(), cmd.genesis_l1_height, ctx)?;
    let alpen_params_path = cmd.alpen_params.as_deref().ok_or_else(|| {
        anyhow!("--alpen-params is required so OL params can include bridge params")
    })?;
    let alpen_params = read_alpen_params(alpen_params_path)?;
    let mut ol_params = OLParams::new_empty(anchor.block, *alpen_params.bridge_params());

    let acct_predicate = resolve_acct_predicate(cmd.alpen_predicate)?;
    let balance = BitcoinAmount::from_sat(cmd.alpen_balance.unwrap_or(0));
    let inner_state = resolve_inner_state(cmd.alpen_inner_state.as_deref(), &alpen_params)?;
    let alpen_ee_account = GenesisSnarkAccountData {
        predicate: acct_predicate,
        inner_state,
        balance,
    };
    let account_id = alpen_params.account_id();
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

fn read_alpen_params(path: &Path) -> anyhow::Result<AlpenParams> {
    let json = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed to read Alpen params file {path:?}: {e}"))?;
    let params = AlpenParams::from_json_str(&json)
        .map_err(|e| anyhow!("failed to parse Alpen params file {path:?}: {e}"))?;
    Ok(params)
}

fn resolve_inner_state(
    alpen_inner_state: Option<&str>,
    alpen_params: &AlpenParams,
) -> anyhow::Result<Buf32> {
    if let Some(hex) = alpen_inner_state {
        return hex
            .parse::<Buf32>()
            .map_err(|e| anyhow!("invalid alpen-inner-state hex: {e}"));
    }

    let genesis_info = alpen_params.genesis_block_info();
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
    use alpen_chainspec::DEV_CHAIN_SPEC;
    use alpen_ee_params::{
        AlpenForkSchedule, AlpenParams, BlobSpec, EvmSpec, DEFAULT_ALPEN_EE_ACCOUNT_ID,
    };
    use strata_bridge_params::BridgeParams;
    use strata_l1_txfmt::MagicBytes;

    use super::{compute_inner_state, resolve_inner_state};

    fn test_alpen_params() -> AlpenParams {
        let evm_spec: EvmSpec = serde_json::from_str(DEV_CHAIN_SPEC).expect("valid dev chain spec");
        AlpenParams::new(
            DEFAULT_ALPEN_EE_ACCOUNT_ID,
            BridgeParams::new_with_descriptor_limit(100_000_000, Some(1_000_000_000), 81)
                .expect("valid bridge params"),
            BlobSpec::new(MagicBytes::new(*b"ALPN")),
            AlpenForkSchedule::default(),
            evm_spec,
        )
    }

    #[test]
    fn explicit_inner_state_overrides_alpen_params() {
        let params = test_alpen_params();
        let expected = "abababababababababababababababababababababababababababababababab"
            .parse()
            .unwrap();

        let resolved = resolve_inner_state(
            Some("abababababababababababababababababababababababababababababababab"),
            &params,
        )
        .unwrap();

        assert_eq!(resolved, expected);
    }

    #[test]
    fn alpen_params_supply_inner_state_when_no_override_is_present() {
        let params = test_alpen_params();
        let resolved = resolve_inner_state(None, &params).unwrap();

        let genesis_info = alpen_chainspec::ee_genesis_block_info_from_json(DEV_CHAIN_SPEC)
            .expect("valid dev chain spec");
        let expected = compute_inner_state(genesis_info.blockhash(), genesis_info.stateroot());

        assert_eq!(resolved, expected);
    }
}
