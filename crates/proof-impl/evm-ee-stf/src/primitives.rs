use std::collections::HashMap;

use alloy_consensus::{serde_bincode_compat as serde_bincode_compat_header, Header};
use alloy_eips::eip4895::Withdrawal;
use alpen_reth_primitives::WithdrawalIntent;
use reth_primitives::TransactionSigned;
use revm_primitives::alloy_primitives::{Address, Bytes, FixedBytes, B256};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use strata_state::block::ExecSegment;

use crate::mpt::{MptNode, StorageEntry};

/// Public Parameters that proof asserts
pub type EvmEeProofOutput = Vec<ExecSegment>;

/// Public Parameters that proof asserts
pub type EvmEeProofInput = Vec<EvmBlockStfInput>;

/// Result of the block execution
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvmBlockStfOutput {
    pub block_idx: u64,
    pub prev_blockhash: FixedBytes<32>,
    pub new_blockhash: FixedBytes<32>,
    pub new_state_root: FixedBytes<32>,
    pub txn_root: FixedBytes<32>,
    pub withdrawal_intents: Vec<WithdrawalIntent>,
    pub deposit_requests: Vec<Withdrawal>,
}

/// Necessary information to prove the execution of a Evm block.
#[serde_as]
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvmBlockStfInput {
    /// The Keccak 256-bit hash of the parent block's header, in its entirety.
    /// N.B. The reason serde_bincode_compat is necessary:
    /// [`serde_bincode_compat`](alloy_consensus::serde_bincode_compat)
    #[serde_as(as = "serde_bincode_compat_header::Header")]
    pub parent_header: Header,

    /// The 160-bit address to which all fees collected from the successful mining of this block
    /// be transferred.
    pub beneficiary: Address,

    /// A scalar value equal to the current limit of gas expenditure per block.
    pub gas_limit: u64,

    /// A scalar value equal to the reasonable output of Unix's time() at this block's inception.
    pub timestamp: u64,

    /// An arbitrary byte array containing data relevant to this block. This must be 32 bytes or
    /// fewer.
    pub extra_data: Bytes,

    /// A 256-bit hash which, combined with the nonce, proves that a sufficient amount of
    /// computation has been carried out on this block.
    pub mix_hash: B256,

    /// Represents the pre-state trie containing account states
    /// expected to be accessed or modified during execution.
    pub pre_state_trie: MptNode,

    /// Represents the pre-state storage containing entries
    /// expected to be accessed or modified during execution.
    pub pre_state_storage: HashMap<Address, StorageEntry>,

    /// The relevant contracts for the block.
    pub contracts: Vec<Bytes>,

    /// The ancestor headers of the parent block.
    #[serde_as(as = "Vec<serde_bincode_compat_header::Header>")]
    pub ancestor_headers: Vec<Header>,

    /// A list of transactions to process.
    // #[serde_as(as = "Vec<serde_bincode_compat::TransactionSigned>")]
    // https://github.com/paradigmxyz/reth/issues/15751
    pub transactions: Vec<TransactionSigned>,

    /// A list of withdrawals to process.
    pub withdrawals: Vec<Withdrawal>,
}
