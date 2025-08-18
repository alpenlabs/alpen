//! Bridge transaction data structures
//!
//! This module contains the core data structures for bridge operations,
//! adapted from the mock-bridge implementation for use in python-utils.

use bdk_wallet::bitcoin::{
    consensus, opcodes::all::OP_RETURN, script::{Builder, PushBytesBuf},
    Amount, OutPoint, Psbt, ScriptBuf, TapNodeHash, Transaction, TxOut, Txid,
    XOnlyPublicKey,
};
use pyo3::prelude::*;

/// Data structure for a deposit request transaction (DRT). This is exposed to Python
#[derive(Debug, Clone)]
#[pyclass]
pub(crate) struct DepositRequestTransaction {
    pub transaction: Vec<u8>,
    pub deposit_request_data: DepositRequestData,
}

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

/// Internal Rust representation of DepositRequestData with proper Bitcoin types
#[derive(Debug, Clone)]
pub(crate) struct DepositRequestDataInternal {
    pub deposit_request_outpoint: OutPoint,
    pub stake_index: u32,
    pub el_address: [u8; 20],
    pub total_amount: Amount,
    pub x_only_public_key: XOnlyPublicKey,
    pub original_script_pubkey: ScriptBuf,
}


/// Deposit Transaction structure with PSBT and metadata
#[derive(Debug, Clone)]
pub(crate) struct DepositTx {
    psbt: Psbt,
    prevouts: Vec<TxOut>,
    witnesses: Vec<TaprootWitness>,
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

/// Auxiliary data for deposit transactions
#[derive(Debug, Clone)]
pub(crate) struct AuxiliaryData {
    pub tag: String,
    pub metadata: DepositTxMetadata,
}

/// Taproot witness types adapted from mock-bridge
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaprootWitness {
    /// Use the keypath spend
    Key,

    /// Use the script path spend with script and control block
    Script {
        script_buf: ScriptBuf,
        control_block: bdk_wallet::bitcoin::taproot::ControlBlock,
    },

    /// Use the keypath spend tweaked with some known hash
    Tweaked {
        tweak: TapNodeHash,
    },
}

// DepositTx implementations
impl DepositTx {
    pub(crate) fn new(
        psbt: Psbt,
        prevouts: Vec<TxOut>,
        witnesses: Vec<TaprootWitness>,
    ) -> Self {
        Self {
            psbt,
            prevouts,
            witnesses,
        }
    }

    pub(crate) fn psbt(&self) -> &Psbt {
        &self.psbt
    }

    pub(crate) fn psbt_mut(&mut self) -> &mut Psbt {
        &mut self.psbt
    }

    pub(crate) fn prevouts(&self) -> &[TxOut] {
        &self.prevouts
    }

    pub(crate) fn witnesses(&self) -> &[TaprootWitness] {
        &self.witnesses
    }
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

        push_data.extend_from_slice(&data)
            .expect("metadata should be within push data limits");

        push_data
    }
}

// AuxiliaryData implementations
impl AuxiliaryData {
    pub(crate) fn new(tag: String, metadata: DepositTxMetadata) -> Self {
        Self { tag, metadata }
    }

    pub(crate) fn to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(self.tag.as_bytes());
        buf.extend_from_slice(&self.metadata.stake_index.to_be_bytes());
        buf.extend_from_slice(&self.metadata.ee_address);
        buf.extend_from_slice(&self.metadata.takeback_hash.as_ref());
        buf.extend_from_slice(&self.metadata.input_amount.to_sat().to_be_bytes());
        buf
    }
}
