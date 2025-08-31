//! Bridge transaction data structures
//!
//! This module contains the core data structures for bridge operations,
//! adapted from the mock-bridge implementation for use in python-utils.

use bdk_wallet::bitcoin::{consensus, Amount, TapNodeHash, Txid};
use make_buf::make_buf;

/// Withdrawal fulfillment transaction metadata
#[derive(Debug, Clone)]
pub(crate) struct WithdrawalMetadata {
    /// The tag used to mark the withdrawal metadata transaction
    pub tag: [u8; 4],

    /// The index of the operator
    pub operator_idx: u32,

    /// The index of the deposit
    pub deposit_idx: u32,

    /// The txid of the deposit UTXO
    pub deposit_txid: Txid,
}

// WithdrawalMetadata implementations
impl WithdrawalMetadata {
    pub(crate) fn new(
        tag: [u8; 4],
        operator_idx: u32,
        deposit_idx: u32,
        deposit_txid: Txid,
    ) -> Self {
        Self {
            tag,
            operator_idx,
            deposit_idx,
            deposit_txid,
        }
    }

    pub(crate) fn op_return_data(&self) -> [u8; 44] {
        let deposit_txid_data = consensus::encode::serialize(&self.deposit_txid);
        make_buf! {
            (&self.tag, 4),
            (&self.operator_idx.to_be_bytes(), 4),
            (&self.deposit_idx.to_be_bytes(), 4),
            (&deposit_txid_data, 32),
        }
    }
}

/// Deposit transaction metadata for OP_RETURN
#[derive(Debug, Clone)]
pub(crate) struct DepositTxMetadata {
    pub stake_index: u32,
    pub ee_address: Vec<u8>,
    pub takeback_hash: TapNodeHash,
    pub input_amount: Amount,
}
