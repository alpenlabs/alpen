//! Types for managing pending bridging operations in the CL state.

use std::io::{self, Read, Write};

use bitcoin::Amount;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use strata_identifiers::SubjectId;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

use crate::OperatorSelection;

/// Describes an intent to withdraw that hasn't been dispatched yet.
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
pub struct WithdrawalIntent {
    /// Quantity of L1 asset, for Bitcoin this is sats.
    amt: BitcoinAmount,

    /// Destination [`Descriptor`] for the withdrawal
    destination: Descriptor,

    /// User's operator selection for withdrawal assignment.
    #[borsh(
        serialize_with = "serialize_operator_selection",
        deserialize_with = "deserialize_operator_selection"
    )]
    selected_operator: OperatorSelection,
}

// ASM no longer implements Borsh for OperatorSelection. Keep OL's persisted
// field as the same raw u32, including u32::MAX for "any operator".
fn serialize_operator_selection<W: Write>(
    selection: &OperatorSelection,
    writer: &mut W,
) -> io::Result<()> {
    BorshSerialize::serialize(&selection.raw(), writer)
}

fn deserialize_operator_selection<R: Read>(reader: &mut R) -> io::Result<OperatorSelection> {
    u32::deserialize_reader(reader).map(OperatorSelection::from_raw)
}

impl WithdrawalIntent {
    pub fn new(
        amt: BitcoinAmount,
        destination: Descriptor,
        selected_operator: OperatorSelection,
    ) -> Self {
        Self {
            amt,
            destination,
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
        let total = self
            .intents
            .iter()
            .fold(0u64, |acc, wi| acc.saturating_add(wi.amt.to_sat()));
        // Retain the prior u64 saturation semantics for the aggregate.
        Amount::from_sat(total).into()
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
    use bitcoin::Amount;
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

    use super::{WithdrawalBatch, WithdrawalIntent};
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
        fn withdrawal_intent_encoding_roundtrip(
            amt in 0..=Amount::MAX_MONEY.to_sat(),
            destination in descriptor_strategy(),
            selected_operator in operator_selection_strategy(),
        ) {
            let intent = WithdrawalIntent::new(
                BitcoinAmount::try_from(amt).unwrap(),
                destination,
                selected_operator,
            );

            let encoded = intent.as_ssz_bytes();
            let decoded = WithdrawalIntent::from_ssz_bytes(&encoded).unwrap();

            prop_assert_eq!(&decoded, &intent);

            let encoded = borsh::to_vec(&intent).unwrap();
            let decoded: WithdrawalIntent = borsh::from_slice(&encoded).unwrap();
            prop_assert_eq!(decoded, intent);
        }
    }

    #[test]
    fn withdrawal_batch_total_preserves_saturating_sum() {
        for (amounts, expected) in [
            (vec![], 0),
            (vec![1_000, 2_000], 3_000),
            (
                vec![Amount::MAX_MONEY.to_sat(), 1],
                Amount::MAX_MONEY.to_sat() + 1,
            ),
            (vec![u64::MAX, 1], u64::MAX),
        ] {
            let intents = amounts
                .into_iter()
                .map(|sats| {
                    WithdrawalIntent::new(
                        Amount::from_sat(sats).into(),
                        Descriptor::new_p2wpkh(&[0; 20]),
                        OperatorSelection::any(),
                    )
                })
                .collect();
            assert_eq!(
                WithdrawalBatch::new(intents).get_total_value().to_sat(),
                expected
            );
        }
    }

    #[test]
    fn withdrawal_intent_ssz_rejects_invalid_descriptor_bytes() {
        let encoded = (
            BitcoinAmount::try_from(42).unwrap(),
            vec![0xFFu8; 3],
            OperatorSelection::any(),
        )
            .as_ssz_bytes();

        let err = WithdrawalIntent::from_ssz_bytes(&encoded).unwrap_err();

        assert!(matches!(err, ssz::DecodeError::BytesInvalid(_)));
    }
}
