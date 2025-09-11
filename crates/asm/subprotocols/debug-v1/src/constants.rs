use strata_l1_txfmt::{SubprotocolId, TxType};

/// The unique identifier for the Debug subprotocol within the Anchor State Machine.
///
/// This constant is set to the maximum u8 value (255) to avoid conflicts with production
/// subprotocols, which are assigned incremental IDs starting from 0 as specified in the
/// spec documents. The debug subprotocol is only available when using `DebugAsmSpec`
/// and should not be included in non-testing runtime builds.
pub(crate) const DEBUG_SUBPROTOCOL_ID: SubprotocolId = u8::MAX;

/// Transaction type for sending arbitrary log messages to the Orchestration Layer through the ASM.
///
/// This transaction type enables injection of arbitrary log messages into the ASM log output,
/// simulating logs that would normally originate from the bridge subprotocol.
/// Example: Deposit events (locking funds in n/n multisig)
pub(crate) const FAKE_ASM_LOG_TX_TYPE: TxType = 1;

/// Transaction type for fake withdrawal intent creation.
///
/// This transaction type enables creation of withdrawal intents that are sent to
/// the bridge subprotocol, simulating withdrawals from the Orchestration Layer.
/// Example: Withdrawal Intent (requesting funds from the operators network)
/// These messages normally originate from the Checkpointing subprotocol through inter-protocol
/// messaging.
pub(crate) const FAKE_WITHDRAW_INTENT_TX_TYPE: TxType = 2;
