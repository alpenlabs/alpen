//! `gen-alpen-params` subcommand: generates the Alpen params artifact from inputs.

use std::{fs, path::Path};

use alpen_chainspec::DEV_CHAIN_SPEC;
use alpen_ee_params::{
    AlpenForkSchedule, AlpenParams, BlobSpec, EvmSpec, DEFAULT_ALPEN_EE_ACCOUNT_ID,
};
use strata_identifiers::AccountId;
use strata_l1_txfmt::MagicBytes;
use strata_ol_params::BridgeParams;

use crate::args::{CmdContext, SubcAlpenParams};

/// Default EVM chain spec used when `--alpen-chain-config` is omitted.
const DEFAULT_CHAIN_SPEC: &str = DEV_CHAIN_SPEC;

/// Default EE DA stream magic bytes used when `--da-magic-bytes` is omitted.
const DEFAULT_DA_MAGIC_BYTES: &str = "ALPN";

/// Executes the `gen-alpen-params` subcommand.
pub(super) fn exec(cmd: SubcAlpenParams, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let account_id = match cmd.account_id {
        Some(account_id) => parse_account_id(&account_id)?,
        None => DEFAULT_ALPEN_EE_ACCOUNT_ID,
    };

    let genesis_json = read_chain_config(cmd.alpen_chain_config.as_deref())?;
    let evm_spec: EvmSpec = serde_json::from_str(&genesis_json)
        .map_err(|e| anyhow::anyhow!("invalid EVM chain config: {e}"))?;

    let bridge_params = BridgeParams::new_with_descriptor_limit(
        cmd.bridge_denomination_sats,
        cmd.max_withdrawal_amount_sats,
        cmd.max_withdrawal_descriptor_len,
    )?;

    let magic_bytes: MagicBytes = cmd
        .da_magic_bytes
        .as_deref()
        .unwrap_or(DEFAULT_DA_MAGIC_BYTES)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid DA magic bytes: {e}"))?;

    let params = AlpenParams::new(
        account_id,
        bridge_params,
        BlobSpec::new(magic_bytes),
        AlpenForkSchedule::default(),
        evm_spec,
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
