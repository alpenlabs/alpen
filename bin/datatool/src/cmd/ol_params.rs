//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::anyhow;
use strata_identifiers::AccountId;
use strata_ol_params::{BridgeParams, GenesisSnarkAccountData, OLParams};

use crate::{
    args::{CmdContext, SubcOlParams},
    cmd::genesis_info::retrieve_l1_anchor,
};

/// Executes the `gen-ol-params` subcommand.
///
/// Generates the OL params for a Strata network by retrieving the genesis L1
/// anchor and constructing an [`OLParams`] from the bridge params and the
/// genesis snark accounts. Outputs the result as pretty-printed JSON, either to
/// the specified file or to stdout.
///
/// Genesis snark accounts are supplied whole via `--genesis-accounts`. Their
/// inner state roots and predicates are computed by whoever owns the account's
/// execution environment — the OL only pre-registers them. For the Alpen EE
/// account that generator lives in the alpen-ee repo; this repo carries a
/// committed copy of its output under `.github/fixtures/`.
pub(super) fn exec(cmd: SubcOlParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let anchor = retrieve_l1_anchor(cmd.l1_anchor_file.as_deref(), cmd.genesis_l1_height, ctx)?;
    let bridge_params = BridgeParams::new_with_descriptor_limit(
        cmd.bridge_denomination_sats,
        cmd.max_withdrawal_amount_sats,
        cmd.max_withdrawal_descriptor_len,
    )?;
    let mut ol_params = OLParams::new_empty(anchor.block, bridge_params);

    if let Some(path) = cmd.genesis_accounts.as_deref() {
        for (account_id, account) in read_genesis_accounts(path)? {
            ol_params.insert_genesis_account(account_id, account);
        }
    }

    let params_buf = serde_json::to_string_pretty(&ol_params)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, &params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}

fn read_genesis_accounts(
    path: &Path,
) -> anyhow::Result<BTreeMap<AccountId, GenesisSnarkAccountData>> {
    let json = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed to read genesis accounts file {path:?}: {e}"))?;
    serde_json::from_str(&json)
        .map_err(|e| anyhow!("failed to parse genesis accounts file {path:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use strata_predicate::{PredicateKey, PredicateTypeId};

    use super::read_genesis_accounts;

    const ACCOUNT_ID: &str = "0101010101010101010101010101010101010101010101010101010101010101";
    const INNER_STATE: &str = "abababababababababababababababababababababababababababababababab";

    #[test]
    fn reads_a_genesis_account_entry() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("genesis-accounts.json");
        fs::write(
            &path,
            format!(
                r#"{{"{ACCOUNT_ID}":{{"predicate":"Sp1Groth16:deadbeef","inner_state":"{INNER_STATE}","balance":7}}}}"#
            ),
        )
        .unwrap();

        let accounts = read_genesis_accounts(&path).unwrap();

        assert_eq!(accounts.len(), 1);
        let account = accounts.values().next().unwrap();
        assert_eq!(
            account.predicate,
            PredicateKey::try_new(PredicateTypeId::Sp1Groth16, vec![0xde, 0xad, 0xbe, 0xef])
                .expect("predicate condition must fit within the maximum length")
        );
        assert_eq!(account.inner_state, INNER_STATE.parse().unwrap());
        assert_eq!(account.balance.to_sat(), 7);
    }

    #[test]
    fn rejects_a_malformed_genesis_account_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("genesis-accounts.json");
        fs::write(&path, "{").unwrap();

        assert!(read_genesis_accounts(&path).is_err());
    }
}
