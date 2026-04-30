//! Computes the bridge deposit-request transaction outputs.
//!
//! A DRT (Deposit Request Transaction) is built by the depositor and has two
//! required outputs:
//! - Output 0: SPS-50 OP_RETURN with magic + subproto + tx_type + recovery_pk
//!   + encoded `DepositDescriptor`.
//! - Output 1: P2TR with the operator-aggregated internal key and a takeback
//!   tapscript (recovery_pk + relative timelock).
//!
//! This command derives both pieces from operator extended private keys, a
//! recovery x-only pubkey, and a destination Alpen EE address, and prints
//! them as JSON. The Python functional-test side then constructs and broadcasts
//! the actual DRT via bitcoincli.

use argh::FromArgs;
use bdk_wallet::bitcoin::{Address, Network, secp256k1::SECP256K1};
use serde_json::json;
use strata_asm_proto_bridge_v1_txs::deposit_request::{
    DrtHeaderAux, create_deposit_request_locking_script,
};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_crypto::aggregate_schnorr_keys;
use strata_identifiers::{AccountSerial, SubjectIdBytes, SYSTEM_RESERVED_ACCTS};
use strata_l1_txfmt::ParseConfig;
use strata_ol_bridge_types::DepositDescriptor;
use strata_primitives::{buf::Buf32, constants::RECOVER_DELAY};

use crate::{
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES},
    parse::parse_operator_xprivs,
};

/// Compute the DRT bridge_in P2TR address and SPS-50 OP_RETURN script.
///
/// Output: JSON object with `bridge_in_address`, `op_return_hex`, `amount_sats`.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "compute-drt-output")]
pub struct ComputeDrtOutputArgs {
    /// operator BIP32 extended private keys, JSON array of 78-byte hex strings
    #[argh(option)]
    pub operator_keys: String,

    /// recovery x-only public key, hex (32 bytes)
    #[argh(option)]
    pub recovery_pubkey: String,

    /// alpen EE address (20-byte EVM address), hex
    #[argh(option)]
    pub alpen_address: String,

    /// destination account serial (default: SYSTEM_RESERVED_ACCTS = 128)
    #[argh(option, default = "SYSTEM_RESERVED_ACCTS")]
    pub account_serial: u32,

    /// bitcoin network: regtest, signet, testnet, or mainnet (default: regtest)
    #[argh(option, default = "default_network()")]
    pub network: String,
}

fn default_network() -> String {
    "regtest".to_string()
}

pub(crate) fn compute_drt_output(args: ComputeDrtOutputArgs) -> Result<(), DisplayedError> {
    let keys: Vec<String> =
        serde_json::from_str(&args.operator_keys).user_error("invalid operator_keys JSON")?;

    let signers = parse_operator_xprivs(&keys)
        .user_error("invalid operator key (need base58 xpriv or 78-byte hex)")?;
    let pubkeys: Vec<Buf32> = signers
        .iter()
        .map(|kp| Buf32::from(kp.x_only_public_key(SECP256K1).0.serialize()))
        .collect();

    let internal_key = aggregate_schnorr_keys(pubkeys.iter())
        .internal_error("failed to aggregate operator pubkeys")?;

    let recovery_pk_bytes =
        hex::decode(&args.recovery_pubkey).user_error("invalid recovery_pubkey hex")?;
    let recovery_pk: [u8; 32] = recovery_pk_bytes.try_into().map_err(|_| {
        DisplayedError::UserError(
            "recovery_pubkey must be exactly 32 bytes".to_string(),
            Box::new(()),
        )
    })?;

    let network = parse_network(&args.network)?;

    let bridge_in_script =
        create_deposit_request_locking_script(&recovery_pk, internal_key, RECOVER_DELAY);
    let bridge_in_address = Address::from_script(&bridge_in_script, network)
        .internal_error("failed to derive bridge_in address from script")?;

    let alpen_addr_bytes =
        hex::decode(&args.alpen_address).user_error("invalid alpen_address hex")?;
    let subject = SubjectIdBytes::try_new(alpen_addr_bytes).ok_or_else(|| {
        DisplayedError::UserError(
            "alpen_address subject too long".to_string(),
            Box::new(()),
        )
    })?;

    let descriptor = DepositDescriptor::new(AccountSerial::new(args.account_serial), subject)
        .map_err(|e| {
            DisplayedError::UserError(
                format!("failed to build deposit descriptor: {e}"),
                Box::new(()),
            )
        })?;

    let header_aux = DrtHeaderAux::new(recovery_pk, descriptor.encode_to_varvec())
        .internal_error("failed to build DRT header aux")?;

    let drt_tag = header_aux.build_tag_data();
    let op_return_script = ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&drt_tag.as_ref())
        .internal_error("failed to encode SPS-50 OP_RETURN")?;

    let result = json!({
        "bridge_in_address": bridge_in_address.to_string(),
        "op_return_hex": hex::encode(op_return_script.as_bytes()),
        "amount_sats": BRIDGE_OUT_AMOUNT.to_sat(),
    });

    println!("{result}");
    Ok(())
}

fn parse_network(s: &str) -> Result<Network, DisplayedError> {
    match s {
        "regtest" => Ok(Network::Regtest),
        "signet" => Ok(Network::Signet),
        "testnet" => Ok(Network::Testnet),
        "mainnet" => Ok(Network::Bitcoin),
        other => Err(DisplayedError::UserError(
            format!("unsupported network: {other}"),
            Box::new(()),
        )),
    }
}
