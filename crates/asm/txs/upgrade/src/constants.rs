/// Transaction type 0 (CANCEL) for the Upgrade Subprotocol (SPS-50, Subprotocol ID 0).
/// Signals the cancellation of a previously queued.
pub const CANCEL_TX_TYPE: u8 = 0;

/// Transaction type 1 (ENACT) for the Upgrade Subprotocol (SPS-50, Subprotocol ID 0).
/// Executes (enacts) the upgrade that was previously committed.
pub const ENACT_TX_TYPE: u8 = 1;

/// Transaction type 10 (MULTISIG_CONFIG_UPDATE) for the Upgrade Subprotocol (SPS-50, Subprotocol ID
/// 0). Proposes an update to the on-chain multisignature configuration.
pub const MULTISIG_CONFIG_UPDATE_TX_TYPE: u8 = 10;

/// Transaction type 11 (OPERATOR_UPDATE) for the Upgrade Subprotocol (SPS-50, Subprotocol ID 0).
/// Proposes an update to the set of authorized operators.
pub const OPERATOR_UPDATE_TX_TYPE: u8 = 11;

/// Transaction type 12 (SEQUENCER_UPDATE) for the Upgrade Subprotocol (SPS-50, Subprotocol ID 0).
/// Proposes an update to the sequencer configuration.
pub const SEQUENCER_UPDATE_TX_TYPE: u8 = 12;

/// Transaction type 13 (VK_UPDATE) for the Upgrade Subprotocol (SPS-50, Subprotocol ID 0).
/// Proposes an update to the verifying key used by the protocol.
pub const VK_UPDATE_TX_TYPE: u8 = 13;
