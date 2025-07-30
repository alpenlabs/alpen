use bitcoin::Transaction;

use crate::txs::{deposit::DepositInfo, withdrawal_fulfillment::WithdrawalFulfillmentInfo};

pub(crate) mod deposit;
mod handler;
mod parser;
pub(crate) mod withdrawal_fulfillment;

pub(crate) use handler::handle_parsed_tx;
pub(crate) use parser::parse_tx;

/// A parsed deposit transaction containing the raw transaction and extracted deposit information.
#[derive(Debug)]
pub(crate) struct ParsedDepositTx<'t> {
    pub tx: &'t Transaction,
    pub info: DepositInfo,
}

/// A parsed withdrawal fulfillment transaction containing the raw transaction and extracted
/// withdrawal information.
#[derive(Debug)]
pub(crate) struct ParsedWithdrawalFulfillmentTx<'t> {
    pub tx: &'t Transaction,
    pub info: WithdrawalFulfillmentInfo,
}

/// Represents a parsed transaction that can be either a deposit or withdrawal fulfillment.
#[derive(Debug)]
pub(crate) enum ParsedTx<'t> {
    /// A deposit transaction that locks Bitcoin funds in the bridge
    Deposit(ParsedDepositTx<'t>),
    /// A withdrawal fulfillment transaction that releases Bitcoin funds from the bridge
    WithdrawalFulfillment(ParsedWithdrawalFulfillmentTx<'t>),
}
