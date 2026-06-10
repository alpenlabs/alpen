//! `gen-ee-params` subcommand: generates Alpen EE params from inputs.

use std::{fs, path::Path};

use alpen_chainspec::{ee_genesis_block_info_from_json, DEV_CHAIN_SPEC};
use alpen_ee_config::{AlpenEeParams, DEFAULT_ALPEN_EE_ACCOUNT_ID};
use strata_identifiers::AccountId;

use crate::args::{CmdContext, SubcEeParams};

/// Default EVM chain spec used when `--alpen-chain-config` is omitted.
const DEFAULT_CHAIN_SPEC: &str = DEV_CHAIN_SPEC;

/// Executes the `gen-ee-params` subcommand.
pub(super) fn exec(cmd: SubcEeParams, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let account_id = match cmd.account_id {
        Some(account_id) => parse_account_id(&account_id)?,
        None => DEFAULT_ALPEN_EE_ACCOUNT_ID,
    };

    let genesis_json = read_chain_config(cmd.alpen_chain_config.as_deref())?;
    let genesis_info = ee_genesis_block_info_from_json(&genesis_json)?;
    let params = AlpenEeParams::new(
        account_id,
        genesis_info.blockhash(),
        genesis_info.stateroot(),
        genesis_info.blocknum(),
    );
    let params_buf = params.to_json_string_pretty()?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, &params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}

/// Reads the EVM chain config JSON or returns the built-in dev chain spec.
pub(super) fn read_chain_config(chain_config: Option<&Path>) -> anyhow::Result<String> {
    match chain_config {
        Some(path) => fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read chain config {path:?}: {e}")),
        None => Ok(DEFAULT_CHAIN_SPEC.into()),
    }
}

/// Parses a 32-byte account id from a 64-character hex string.
pub(super) fn parse_account_id(account_id: &str) -> anyhow::Result<AccountId> {
    let account_id = account_id.trim();
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(account_id, &mut bytes)
        .map_err(|e| anyhow::anyhow!("invalid account id hex: {e}"))?;
    Ok(AccountId::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::parse_account_id;

    #[test]
    fn parses_64_char_account_id_hex() {
        let parsed =
            parse_account_id("0101010101010101010101010101010101010101010101010101010101010101")
                .expect("account id should parse");

        assert_eq!(parsed.into_inner(), [1u8; 32]);
    }

    #[test]
    fn rejects_short_account_id_hex() {
        assert!(parse_account_id("01").is_err());
    }
}
