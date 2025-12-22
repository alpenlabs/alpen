use strata_l1_txfmt::SubprotocolId;

/// The unique identifier for the Bridge V1 subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the Bridge V1 logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const BRIDGE_V1_SUBPROTOCOL_ID: SubprotocolId = 2;

/// Bridge V1 transaction types.
///
/// This enum represents all valid transaction types for the Bridge V1 subprotocol.
/// Each variant corresponds to a specific transaction type with its associated u8 value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BridgeTxType {
    /// Deposit request transaction - user initiates a deposit
    DepositRequest = 0,
    /// Deposit transaction - operator accepts the deposit
    Deposit = 1,
    /// Withdrawal fulfillment transaction - operator fulfills withdrawal
    WithdrawalFulfillment = 2,
    /// Commit transaction - operator commits to a game
    Commit = 3,
    /// Slash transaction - penalize misbehaving operator
    Slash = 4,
    /// Unstake transaction - operator exits the bridge
    Unstake = 5,
}

impl From<BridgeTxType> for u8 {
    fn from(tx_type: BridgeTxType) -> Self {
        tx_type as u8
    }
}

impl TryFrom<u8> for BridgeTxType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(BridgeTxType::DepositRequest),
            1 => Ok(BridgeTxType::Deposit),
            2 => Ok(BridgeTxType::WithdrawalFulfillment),
            3 => Ok(BridgeTxType::Commit),
            4 => Ok(BridgeTxType::Slash),
            5 => Ok(BridgeTxType::Unstake),
            invalid => Err(invalid),
        }
    }
}

impl std::fmt::Display for BridgeTxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeTxType::DepositRequest => write!(f, "DepositRequest"),
            BridgeTxType::Deposit => write!(f, "Deposit"),
            BridgeTxType::WithdrawalFulfillment => write!(f, "WithdrawalFulfillment"),
            BridgeTxType::Commit => write!(f, "Commit"),
            BridgeTxType::Slash => write!(f, "Slash"),
            BridgeTxType::Unstake => write!(f, "Unstake"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for BridgeTxType {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(BridgeTxType::DepositRequest),
                Just(BridgeTxType::Deposit),
                Just(BridgeTxType::WithdrawalFulfillment),
                Just(BridgeTxType::Commit),
                Just(BridgeTxType::Slash),
                Just(BridgeTxType::Unstake),
            ]
            .boxed()
        }
    }

    #[test]
    fn test_bridge_tx_type_discriminants() {
        // Explicitly verify the discriminant values are correct
        assert_eq!(BridgeTxType::DepositRequest as u8, 0);
        assert_eq!(BridgeTxType::Deposit as u8, 1);
        assert_eq!(BridgeTxType::WithdrawalFulfillment as u8, 2);
        assert_eq!(BridgeTxType::Commit as u8, 3);
        assert_eq!(BridgeTxType::Slash as u8, 4);
        assert_eq!(BridgeTxType::Unstake as u8, 5);
    }

    proptest! {
        #[test]
        fn test_bridge_tx_type_roundtrip(tx_type: BridgeTxType) {
            // Test that converting to u8 and back preserves the value
            let as_u8: u8 = tx_type.into();
            let back_to_enum = BridgeTxType::try_from(as_u8)
                .expect("roundtrip conversion should succeed");
            prop_assert_eq!(tx_type, back_to_enum);
        }

        #[test]
        fn test_bridge_tx_type_invalid_values(value in 6u8..=255u8) {
            // Test that all invalid u8 values return an error
            prop_assert!(BridgeTxType::try_from(value).is_err());
        }
    }
}
