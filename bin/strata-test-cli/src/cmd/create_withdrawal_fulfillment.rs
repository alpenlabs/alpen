use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::bridge::{types::BitcoinDConfig, withdrawal};

/// Arguments for creating a withdrawal fulfillment transaction (WFT).
///
/// Creates a Bitcoin transaction that fulfills a withdrawal request from the bridge.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-withdrawal-fulfillment")]
pub struct CreateWithdrawalFulfillmentArgs {
    /// destination Bitcoin address (BOSD format)
    #[argh(option)]
    pub destination: String,

    /// amount in satoshis
    #[argh(option)]
    pub amount: u64,

    /// operator index
    #[argh(option)]
    pub operator_idx: u32,

    /// deposit index
    #[argh(option)]
    pub deposit_idx: u32,

    /// deposit transaction ID (hex)
    #[argh(option)]
    pub deposit_txid: String,

    /// bitcoin RPC URL
    #[argh(option)]
    pub btc_url: String,

    /// bitcoin RPC username
    #[argh(option)]
    pub btc_user: String,

    /// bitcoin RPC password
    #[argh(option)]
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
