use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use digest::Digest;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::{
    batch::SignedCheckpoint,
    buf::Buf32,
    l1::{BitcoinAmount, OutputRef},
};

/// Commits to a DA blob.  This is just the hash of the DA blob.
#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Arbitrary,
    Serialize,
    Deserialize,
)]
pub struct DaCommitment(Buf32);

impl DaCommitment {
    /// Creates a commitment from a DA payload buf.
    pub fn from_buf(buf: &[u8]) -> Self {
        Self::from_chunk_iter([buf].into_iter())
    }

    /// Creates a commitment from a series of contiguous chunks of a single DA
    /// paylod buf.
    ///
    /// This is meant to be used when constructing a commitment from an in-situ
    /// payload from a transaction, which has to be in 520-byte chunks.
    pub fn from_chunk_iter<'a>(chunks: impl Iterator<Item = &'a [u8]>) -> Self {
        // TODO maybe abstract this further?
        let mut hasher = Sha256::new();
        for chunk in chunks {
            hasher.update(chunk);
        }

        let hash: [u8; 32] = hasher.finalize().into();
        Self(Buf32(hash))
    }

    pub fn as_hash(&self) -> &Buf32 {
        &self.0
    }

    pub fn to_hash(&self) -> Buf32 {
        self.0
    }
}

/// Consensus level protocol operations extracted from a bitcoin transaction.
///
/// These are submitted to the OL STF and impact state.
#[derive(
    Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
#[allow(clippy::large_enum_variant)]
pub enum ProtocolOperation {
    /// Deposit Transaction
    Deposit(DepositInfo),

    /// Checkpoint data
    Checkpoint(SignedCheckpoint),

    /// DA blob
    DaCommitment(DaCommitment),

    /// Deposit request.
    ///
    /// This is being removed soon as it's not really a consensus change.
    DepositRequest(DepositRequestInfo),

    /// Withdrawal fulfilled by bridge operator front-payment.
    WithdrawalFulfillment(WithdrawalFulfillmentInfo),

    /// Deposit utxo is spent.
    DepositSpent(DepositSpendInfo),
}

#[derive(
    Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub struct DepositInfo {
    /// Deposit from tag output, as assigned by operators.
    pub deposit_idx: u32,

    /// Bitcoin amount.
    pub amt: BitcoinAmount,

    /// Output for deposit funds at rest.
    pub outpoint: OutputRef,

    /// Destination address payload.
    pub address: Vec<u8>,
}

#[derive(
    Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub struct DepositRequestInfo {
    /// amount in satoshis
    pub amt: u64,

    /// tapscript control block hash for timelock script
    pub take_back_leaf_hash: [u8; 32],

    /// EE address
    pub address: Vec<u8>,
}

#[derive(
    Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub struct WithdrawalFulfillmentInfo {
    /// index of deposit this fulfillment is for
    pub deposit_idx: u32,

    /// assigned operator
    /// TODO: maybe this is not needed
    pub operator_idx: u32,

    /// amount that was actually sent on bitcoin.
    /// should equal withdrawal_amount - operator fee
    pub amt: BitcoinAmount,

    /// corresponding bitcoin transaction id.
    pub txid: Buf32,
}

#[derive(
    Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub struct DepositSpendInfo {
    /// index of the deposit whose utxo is spent.
    pub deposit_idx: u32,
}
