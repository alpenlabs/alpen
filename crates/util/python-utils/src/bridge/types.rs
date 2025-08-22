//! Bridge transaction data structures
//!
//! This module contains the core data structures for bridge operations,
//! adapted from the mock-bridge implementation for use in python-utils.

use bdk_wallet::bitcoin::{
    consensus, script::PushBytesBuf, Amount, OutPoint, ScriptBuf, TapNodeHash, Txid, XOnlyPublicKey,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DepositRequestData {
    /// The deposit request transaction outpoints from the users.
    pub deposit_request_outpoint: OutPoint,

    /// The stake index that will be tied to this deposit.
    ///
    /// This is required in order to make sure that the at withdrawal time, deposit UTXOs are
    /// assigned in the same order that the stake transactions were linked during setup time
    ///
    /// # Note
    ///
    /// The stake index must be encoded in 4-byte big-endian.
    pub stake_index: u32,

    /// The execution environment address to mint the equivalent tokens to.
    /// As of now, this is just the 20-byte EVM address.
    pub ee_address: Vec<u8>,

    /// The amount in bitcoins that the user is sending.
    ///
    /// This amount should be greater than the bridge denomination for the deposit to be
    /// confirmed on bitcoin. The excess amount is used as miner fees for the Deposit Transaction.
    pub total_amount: Amount,

    /// The [`XOnlyPublicKey`] in the Deposit Request Transaction (DRT) as provided by the
    /// user in their `OP_RETURN` output.
    pub x_only_public_key: XOnlyPublicKey,

    /// The original script_pubkey in the Deposit Request Transaction (DRT) output used to sanity
    /// check computation internally i.e., whether the known information (n/n script spend path,
    /// [`static@UNSPENDABLE_INTERNAL_KEY`]) + the [`Self::take_back_leaf_hash`] yields the same
    /// P2TR address.
    pub original_script_pubkey: ScriptBuf,
}

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

/// Deposit transaction metadata for OP_RETURN
#[derive(Debug, Clone)]
pub(crate) struct DepositTxMetadata {
    pub stake_index: u32,
    pub ee_address: Vec<u8>,
    pub takeback_hash: TapNodeHash,
    pub input_amount: Amount,
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

    pub(crate) fn op_return_data(&self) -> Vec<u8> {
        let op_id_prefix: [u8; 4] = self.operator_idx.to_be_bytes();
        let deposit_id_prefix: [u8; 4] = self.deposit_idx.to_be_bytes();
        let deposit_txid_data = consensus::encode::serialize(&self.deposit_txid);
        [
            &self.tag[..],
            &op_id_prefix[..],
            &deposit_id_prefix[..],
            &deposit_txid_data[..],
        ]
        .concat()
    }

    pub(crate) fn op_return_script(&self) -> PushBytesBuf {
        let data = self.op_return_data();
        let mut push_data = PushBytesBuf::new();

        push_data
            .extend_from_slice(&data)
            .expect("metadata should be within push data limits");

        push_data
    }
}
