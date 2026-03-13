//! Types for managing pending bridging operations in the CL state.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_identifiers::SubjectId;
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32, l1::BitcoinAmount};

use crate::OperatorSelection;

/// Describes an intent to withdraw that hasn't been dispatched yet.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct WithdrawalIntent {
    /// Quantity of L1 asset, for Bitcoin this is sats.
    amt: BitcoinAmount,

    /// Destination [`Descriptor`] for the withdrawal
    destination: Descriptor,

    /// withdrawal request transaction id
    withdrawal_txid: Buf32,

    /// User's operator selection for withdrawal assignment.
    selected_operator: OperatorSelection,
}

#[derive(DeriveEncode, DeriveDecode)]
struct WithdrawalIntentSsz {
    amt: BitcoinAmount,
    destination: Vec<u8>,
    withdrawal_txid: Buf32,
    selected_operator: OperatorSelection,
}

impl WithdrawalIntent {
    pub fn new(
        amt: BitcoinAmount,
        destination: Descriptor,
        withdrawal_txid: Buf32,
        selected_operator: OperatorSelection,
    ) -> Self {
        Self {
            amt,
            destination,
            withdrawal_txid,
            selected_operator,
        }
    }

    pub fn as_parts(&self) -> (u64, &Descriptor) {
        (self.amt.to_sat(), &self.destination)
    }

    pub fn amt(&self) -> &BitcoinAmount {
        &self.amt
    }

    pub fn destination(&self) -> &Descriptor {
        &self.destination
    }

    pub fn withdrawal_txid(&self) -> &Buf32 {
        &self.withdrawal_txid
    }

    pub fn selected_operator(&self) -> OperatorSelection {
        self.selected_operator
    }
}

impl Encode for WithdrawalIntent {
    fn is_ssz_fixed_len() -> bool {
        <WithdrawalIntentSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <WithdrawalIntentSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        WithdrawalIntentSsz {
            amt: self.amt,
            destination: self.destination.to_bytes(),
            withdrawal_txid: self.withdrawal_txid,
            selected_operator: self.selected_operator,
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        WithdrawalIntentSsz {
            amt: self.amt,
            destination: self.destination.to_bytes(),
            withdrawal_txid: self.withdrawal_txid,
            selected_operator: self.selected_operator,
        }
        .ssz_bytes_len()
    }
}

impl Decode for WithdrawalIntent {
    fn is_ssz_fixed_len() -> bool {
        <WithdrawalIntentSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <WithdrawalIntentSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = WithdrawalIntentSsz::from_ssz_bytes(bytes)?;
        let destination = Descriptor::from_bytes(&value.destination)
            .map_err(|err| DecodeError::BytesInvalid(err.to_string()))?;

        Ok(Self {
            amt: value.amt,
            destination,
            withdrawal_txid: value.withdrawal_txid,
            selected_operator: value.selected_operator,
        })
    }
}

/// Set of withdrawals that are assigned to a deposit bridge utxo.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct WithdrawalBatch {
    /// A series of [WithdrawalIntent]'s who sum does not exceed withdrawal denomination.
    intents: Vec<WithdrawalIntent>,
}

impl WithdrawalBatch {
    /// Creates a new instance.
    pub const fn new(intents: Vec<WithdrawalIntent>) -> Self {
        Self { intents }
    }

    /// Gets the total value of the batch.  This must be less than the size of
    /// the utxo it's assigned to.
    pub fn get_total_value(&self) -> BitcoinAmount {
        self.intents
            .iter()
            .fold(BitcoinAmount::ZERO, |acc, wi| acc.saturating_add(wi.amt))
    }

    pub fn intents(&self) -> &[WithdrawalIntent] {
        &self.intents[..]
    }
}

/// Describes a deposit data to be processed by an EE.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct DepositIntent {
    /// Quantity in the L1 asset, for Bitcoin this is sats.
    amt: BitcoinAmount,

    /// Destination subject identifier within the execution environment.
    dest_ident: SubjectId,
}

impl DepositIntent {
    pub const fn new(amt: BitcoinAmount, dest_ident: SubjectId) -> Self {
        Self { amt, dest_ident }
    }

    pub fn amt(&self) -> u64 {
        self.amt.to_sat()
    }

    pub const fn dest_ident(&self) -> &SubjectId {
        &self.dest_ident
    }
}
