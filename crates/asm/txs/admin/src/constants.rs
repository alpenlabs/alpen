use std::fmt;

use strata_asm_common::SubprotocolId;

/// Unique identifier for the Administration Subprotocol.
pub const ADMINISTRATION_SUBPROTOCOL_ID: SubprotocolId = 0;

/// Administration subprotocol transaction types.
///
/// This enum represents all valid transaction types for the Administration subprotocol.
/// Each variant corresponds to a specific transaction type with its associated u8 value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AdminTxType {
    /// Cancel a previously queued update.
    Cancel = 0,
    /// Update the strata admin multisignature configuration.
    StrataAdminMultisigUpdate = 10,
    /// Update the strata seq manager multisignature configuration.
    StrataSeqManagerMultisigUpdate = 11,
    /// Update the set of authorized operators.
    OperatorUpdate = 20,
    /// Update the sequencer configuration.
    SequencerUpdate = 21,
    /// Update the verifying key for the OL STF.
    OlStfVkUpdate = 30,
    /// Update the verifying key for the ASM STF.
    AsmStfVkUpdate = 31,
}

impl From<AdminTxType> for u8 {
    fn from(tx_type: AdminTxType) -> Self {
        tx_type as u8
    }
}

impl TryFrom<u8> for AdminTxType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AdminTxType::Cancel),
            10 => Ok(AdminTxType::StrataAdminMultisigUpdate),
            11 => Ok(AdminTxType::StrataSeqManagerMultisigUpdate),
            20 => Ok(AdminTxType::OperatorUpdate),
            21 => Ok(AdminTxType::SequencerUpdate),
            30 => Ok(AdminTxType::OlStfVkUpdate),
            31 => Ok(AdminTxType::AsmStfVkUpdate),
            invalid => Err(invalid),
        }
    }
}

impl fmt::Display for AdminTxType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdminTxType::Cancel => write!(f, "Cancel"),
            AdminTxType::StrataAdminMultisigUpdate => write!(f, "StrataAdminMultisigUpdate"),
            AdminTxType::StrataSeqManagerMultisigUpdate => {
                write!(f, "StrataSeqManagerMultisigUpdate")
            }
            AdminTxType::OperatorUpdate => write!(f, "OperatorUpdate"),
            AdminTxType::SequencerUpdate => write!(f, "SequencerUpdate"),
            AdminTxType::OlStfVkUpdate => write!(f, "OlStfVkUpdate"),
            AdminTxType::AsmStfVkUpdate => write!(f, "AsmStfVkUpdate"),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    impl Arbitrary for AdminTxType {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(AdminTxType::Cancel),
                Just(AdminTxType::StrataAdminMultisigUpdate),
                Just(AdminTxType::StrataSeqManagerMultisigUpdate),
                Just(AdminTxType::OperatorUpdate),
                Just(AdminTxType::SequencerUpdate),
                Just(AdminTxType::OlStfVkUpdate),
                Just(AdminTxType::AsmStfVkUpdate),
            ]
            .boxed()
        }
    }

    #[test]
    fn test_admin_tx_type_discriminants() {
        assert_eq!(AdminTxType::Cancel as u8, 0);
        assert_eq!(AdminTxType::StrataAdminMultisigUpdate as u8, 10);
        assert_eq!(AdminTxType::StrataSeqManagerMultisigUpdate as u8, 11);
        assert_eq!(AdminTxType::OperatorUpdate as u8, 20);
        assert_eq!(AdminTxType::SequencerUpdate as u8, 21);
        assert_eq!(AdminTxType::OlStfVkUpdate as u8, 30);
        assert_eq!(AdminTxType::AsmStfVkUpdate as u8, 31);
    }

    proptest! {
        #[test]
        fn test_admin_tx_type_roundtrip(tx_type: AdminTxType) {
            let as_u8: u8 = tx_type.into();
            let back_to_enum = AdminTxType::try_from(as_u8)
                .expect("roundtrip conversion should succeed");
            prop_assert_eq!(tx_type, back_to_enum);
        }

        #[test]
        fn test_admin_tx_type_invalid_values(
            value in (0u8..=255u8).prop_filter("must not be a valid variant", |v| {
                !matches!(*v, 0 | 10 | 11 | 20 | 21 | 30 | 31)
            })
        ) {
            prop_assert!(AdminTxType::try_from(value).is_err());
        }
    }
}
