//! Types for managing pending bridging operations in the CL state.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode as SszDecodeTrait, DecodeError, Encode as SszEncodeTrait};
use ssz_derive::{Decode, Encode};
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

impl SszEncodeTrait for WithdrawalIntent {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        (
            self.amt,
            self.destination.to_bytes(),
            self.withdrawal_txid,
            self.selected_operator,
        )
            .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        (
            self.amt,
            self.destination.to_bytes(),
            self.withdrawal_txid,
            self.selected_operator,
        )
            .ssz_bytes_len()
    }
}

impl SszDecodeTrait for WithdrawalIntent {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let (amt, destination, withdrawal_txid, selected_operator): (
            BitcoinAmount,
            Vec<u8>,
            Buf32,
            OperatorSelection,
        ) = <(BitcoinAmount, Vec<u8>, Buf32, OperatorSelection)>::from_ssz_bytes(bytes)?;
        let destination = Descriptor::from_bytes(&destination)
            .map_err(|err| DecodeError::BytesInvalid(err.to_string()))?;

        Ok(Self {
            amt,
            destination,
            withdrawal_txid,
            selected_operator,
        })
    }
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

/// Set of withdrawals that are assigned to a deposit bridge utxo.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
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
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Encode, Decode)]
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32, l1::BitcoinAmount};

    use super::WithdrawalIntent;
    use crate::OperatorSelection;

    fn descriptor_strategy() -> impl Strategy<Value = Descriptor> {
        prop_oneof![
            any::<[u8; 20]>().prop_map(|hash160| Descriptor::new_p2wpkh(&hash160)),
            any::<[u8; 32]>().prop_map(|hash256| Descriptor::new_p2wsh(&hash256)),
        ]
    }

    fn operator_selection_strategy() -> impl Strategy<Value = OperatorSelection> {
        prop_oneof![
            Just(OperatorSelection::any()),
            any::<u32>()
                .prop_filter("u32::MAX is reserved for the 'any' sentinel", |idx| *idx
                    != u32::MAX)
                .prop_map(OperatorSelection::specific),
        ]
    }

    proptest! {
        #[test]
        fn withdrawal_intent_ssz_roundtrip(
            amt in any::<u64>(),
            destination in descriptor_strategy(),
            withdrawal_txid in any::<[u8; 32]>(),
            selected_operator in operator_selection_strategy(),
        ) {
            let intent = WithdrawalIntent::new(
                BitcoinAmount::from_sat(amt),
                destination,
                Buf32::from(withdrawal_txid),
                selected_operator,
            );

            let encoded = intent.as_ssz_bytes();
            let decoded = WithdrawalIntent::from_ssz_bytes(&encoded).unwrap();

            prop_assert_eq!(decoded, intent);
        }
    }

    #[test]
    fn withdrawal_intent_ssz_rejects_invalid_descriptor_bytes() {
        let encoded = (
            BitcoinAmount::from_sat(42),
            vec![0xFFu8; 3],
            Buf32::from([7u8; 32]),
            OperatorSelection::any(),
        )
            .as_ssz_bytes();

        let err = WithdrawalIntent::from_ssz_bytes(&encoded).unwrap_err();

        assert!(matches!(err, ssz::DecodeError::BytesInvalid(_)));
    }
}
