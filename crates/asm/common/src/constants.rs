use crate::SubprotocolId;

/// The unique identifier for the CoreASM subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the CoreASM logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const CORE_SUBPROTOCOL_ID: SubprotocolId = 1;

pub const BRIDGE_SUBPROTOCOL_ID: SubprotocolId = 2;

/// Transaction type identifiers
pub const OL_STF_CHECKPOINT_TX_TYPE: u8 = 1;
pub const FORCED_INCLUSION_TX_TYPE: u8 = 2;
pub const EE_UPGRADE_TX_TYPE: u8 = 3;
