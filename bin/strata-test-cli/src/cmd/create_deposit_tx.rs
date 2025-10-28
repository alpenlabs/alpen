use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::bridge::dt;

/// Arguments for creating a deposit transaction (DT).
///
/// Creates a deposit transaction from a Deposit Request Transaction (DRT) using operator keys.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-deposit-tx")]
pub struct CreateDepositTxArgs {
    #[argh(option)]
    /// raw DRT transaction in hex-encoded string
    pub drt_tx: String,

    #[argh(option)]
    /// operator private keys in JSON array format (each key is 78 bytes hex)
    /// Example: --operator-keys='["foo", "bar"]'
    pub operator_keys: String,

    #[argh(option)]
    /// deposit transaction index
    pub index: u32,
}

pub(crate) fn create_deposit_tx(args: CreateDepositTxArgs) -> Result<(), DisplayedError> {
    let tx_bytes = hex::decode(&args.drt_tx).user_error("Invalid DRT hex-encoded string")?;

    let keys: Vec<String> = serde_json::from_str(&args.operator_keys)
        .user_error("Invalid operator keys JSON format")?;

    let keys_bytes: Result<Vec<[u8; 78]>, DisplayedError> = keys
        .iter()
        .map(|k| {
            let bytes = hex::decode(k).user_error("Invalid operator key hex-encoded string")?;

            if bytes.len() != 78 {
                return Err(DisplayedError::UserError(
                    format!(
                        "Invalid operator key length: expected 78 bytes, got {}",
                        bytes.len()
                    ),
                    Box::new(()),
                ));
            }

            let mut arr = [0u8; 78];
            arr.copy_from_slice(&bytes);
            Ok(arr)
        })
        .collect();
    let keys_bytes = keys_bytes?;

    let result = dt::create_deposit_transaction_cli(tx_bytes, keys_bytes, args.index)
        .internal_error("Failed to create deposit transaction")?;
    println!("{}", hex::encode(result));

    Ok(())
}
