use strata_l1_txfmt::{SubprotocolId, TxType};

/// The unique identifier for the Bridge V1 subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the Bridge V1 logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const BRIDGE_V1_SUBPROTOCOL_ID: SubprotocolId = 2;

pub(crate) const DEPOSIT_TX_TYPE: TxType = 1;
pub(crate) const WITHDRAWAL_TX_TYPE: TxType = 2;
