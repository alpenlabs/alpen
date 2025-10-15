//! Strata Test CLI
//!
//! Command-line utilities for Strata functional tests, providing bridge operations including:
//! - Deposit transaction creation
//! - Withdrawal fulfillment
//! - Schnorr signature operations
//! - Taproot address operations
//! - Public key aggregation (MuSig2)

use std::process;

mod args;
mod bridge;
mod constants;
mod error;
mod parse;
mod schnorr;
mod taproot;
mod utils;

use args::{Args, Command};
use bridge::{dt, withdrawal};
use error::Error;
use schnorr::sign_schnorr_sig_inner;
use taproot::{
    convert_to_xonly_pk_inner, extract_p2tr_pubkey_inner, get_address_inner,
    musig_aggregate_pks_inner,
};
use utils::xonlypk_to_descriptor_inner;

fn main() {
    if let Err(e) = run() {
        eprintln!("strata-test-cli: {}", e);
        process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let args: Args = argh::from_env();

    match args.command {
        Command::CreateDepositTx(args) => {
            let tx_bytes = hex::decode(&args.drt_tx)
                .map_err(|e| Error::InvalidDrtHex(e.to_string()))?;

            let keys: Vec<String> = serde_json::from_str(&args.operator_keys)
                .map_err(|e| Error::InvalidOperatorKeysJson(e.to_string()))?;

            let keys_bytes: Result<Vec<[u8; 78]>, Error> = keys
                .iter()
                .map(|k| {
                    let bytes = hex::decode(k)
                        .map_err(|e| Error::InvalidHex(e.to_string()))?;

                    if bytes.len() != 78 {
                        return Err(Error::InvalidKeyLength(bytes.len()));
                    }

                    let mut arr = [0u8; 78];
                    arr.copy_from_slice(&bytes);
                    Ok(arr)
                })
                .collect();
            let keys_bytes = keys_bytes?;

            let result = dt::create_deposit_transaction_cli(tx_bytes, keys_bytes, args.index)?;
            println!("{}", hex::encode(result));
        }

        Command::CreateWithdrawalFulfillment(args) => {
            let result = withdrawal::create_withdrawal_fulfillment_cli(
                args.destination,
                args.amount,
                args.operator_idx,
                args.deposit_idx,
                args.deposit_txid,
                args.btc_url,
                args.btc_user,
                args.btc_password,
            )?;
            println!("{}", hex::encode(result));
        }

        Command::GetAddress(args) => {
            let address = get_address_inner(args.index)?;
            println!("{}", address);
        }

        Command::MusigAggregatePks(args) => {
            let pks: Vec<String> = serde_json::from_str(&args.pubkeys)
                .map_err(|e| Error::InvalidPubkeysJson(e.to_string()))?;

            let result = musig_aggregate_pks_inner(pks)?;
            println!("{}", result);
        }

        Command::ExtractP2trPubkey(args) => {
            let result = extract_p2tr_pubkey_inner(args.address)?;
            println!("{}", result);
        }

        Command::ConvertToXonlyPk(args) => {
            let result = convert_to_xonly_pk_inner(args.pubkey)?;
            println!("{}", result);
        }

        Command::SignSchnorrSig(args) => {
            let (sig, pk) = sign_schnorr_sig_inner(&args.message, &args.secret_key)?;
            let output = serde_json::json!({
                "signature": hex::encode(sig),
                "public_key": hex::encode(pk)
            });
            println!("{}", output);
        }

        Command::XonlypkToDescriptor(args) => {
            let result = xonlypk_to_descriptor_inner(&args.xonly_pubkey)?;
            println!("{}", result);
        }
    }

    Ok(())
}
