use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::bridge::{types::BitcoinDConfig, withdrawal};

/// Arguments for creating a withdrawal fulfillment transaction.
///
/// Creates a Bitcoin transaction that fulfills a withdrawal request from the bridge.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-withdrawal-fulfillment")]
pub struct CreateWithdrawalFulfillmentArgs {
    #[argh(option)]
    /// destination Bitcoin address (BOSD format)
    pub destination: String,

    #[argh(option)]
    /// amount in satoshis
    pub amount: u64,

    #[argh(option)]
    /// operator index
    pub operator_idx: u32,

    #[argh(option)]
    /// deposit index
    pub deposit_idx: u32,

    #[argh(option)]
    /// deposit transaction ID (hex)
    pub deposit_txid: String,

    #[argh(option)]
    /// bitcoin RPC URL
    pub btc_url: String,

    #[argh(option)]
    /// bitcoin RPC username
    pub btc_user: String,

    #[argh(option)]
    /// bitcoin RPC password
    pub btc_password: String,
}

pub(crate) fn create_withdrawal_fulfillment(
    args: CreateWithdrawalFulfillmentArgs,
) -> Result<(), DisplayedError> {
    let bitcoind_config = BitcoinDConfig {
        bitcoind_url: args.btc_url,
        bitcoind_user: args.btc_user,
        bitcoind_password: args.btc_password,
    };

    let result = withdrawal::create_withdrawal_fulfillment_cli(
        args.destination,
        args.amount,
        args.operator_idx,
        args.deposit_idx,
        args.deposit_txid,
        bitcoind_config,
    )
    .internal_error("Failed to create withdrawal fulfillment transaction")?;
    println!("{}", hex::encode(result));

    Ok(())
}
