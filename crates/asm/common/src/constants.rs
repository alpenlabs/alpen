use strata_l1_txfmt::{SubprotocolId, TxType};

/// The unique identifier for the CoreASM subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the CoreASM logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const CORE_SUBPROTOCOL_ID: SubprotocolId = 1;

/// The unique identifier for the BridgeV1 subprotocol within the Anchor State Machine.
pub const BRIDGE_SUBPROTOCOL_ID: SubprotocolId = 2;

/// Core Subprotocol Transaction type identifiers
pub const OL_STF_CHECKPOINT_TX_TYPE: TxType = 1;
pub const FORCED_INCLUSION_TX_TYPE: TxType = 2;
pub const EE_UPGRADE_TX_TYPE: TxType = 3;
