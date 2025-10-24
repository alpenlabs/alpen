use std::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use digest::Digest;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use strata_checkpoint_types::SignedCheckpoint;
use strata_identifiers::Buf32;

use crate::{BitcoinAmount, BitcoinOutPoint};

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
#[expect(clippy::large_enum_variant, reason = "used for protocol operations")]
#[derive(
    Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
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
    pub outpoint: BitcoinOutPoint,

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
    Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
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

// Custom debug implementation to print txid in little endian
impl fmt::Debug for WithdrawalFulfillmentInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let txid_le = {
            let mut bytes = self.txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        f.debug_struct("WithdrawalFulfillmentInfo")
            .field("deposit_idx", &self.deposit_idx)
            .field("operator_idx", &self.operator_idx)
            .field("amt", &self.amt)
            .field("txid", &txid_le)
            .finish()
    }
}

// Custom display implementation to print txid in little endian
impl fmt::Display for WithdrawalFulfillmentInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let txid_le = {
            let mut bytes = self.txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        write!(
            f,
            "WithdrawalFulfillmentInfo {{ deposit_idx: {}, operator_idx: {}, amt: {:?}, txid: {} }}",
            self.deposit_idx, self.operator_idx, self.amt, txid_le
        )
    }
}
