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
