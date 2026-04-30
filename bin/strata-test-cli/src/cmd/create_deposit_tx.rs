use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{bridge::dt, parse::parse_operator_xprivs};

/// Arguments for creating a deposit transaction (DT).
///
/// Creates a deposit transaction from a Deposit Request Transaction (DRT) using operator keys.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-deposit-tx")]
pub struct CreateDepositTxArgs {
    /// raw DRT transaction in hex-encoded string
    #[argh(option)]
    pub drt_tx: String,

    /// operator extended private keys, JSON array. Each entry can be the BIP32
    /// base58 form (`tprv...` / `xprv...`) or the 78-byte raw key hex-encoded.
    /// Example: `--operator-keys='["tprv8Zg..."]'`
    #[argh(option)]
    pub operator_keys: String,

    /// deposit transaction index
    #[argh(option)]
    pub index: u32,
}

pub(crate) fn create_deposit_tx(args: CreateDepositTxArgs) -> Result<(), DisplayedError> {
    let tx_bytes = hex::decode(&args.drt_tx).user_error("Invalid DRT hex-encoded string")?;

    let keys: Vec<String> = serde_json::from_str(&args.operator_keys)
        .user_error("Invalid operator keys JSON format")?;

    let signers =
        parse_operator_xprivs(&keys).user_error("Invalid operator key (need base58 xpriv or 78-byte hex)")?;

    let result = dt::create_deposit_transaction_cli(tx_bytes, signers, args.index)
        .internal_error("Failed to create deposit transaction")?;
    println!("{}", hex::encode(result));

    Ok(())
}
