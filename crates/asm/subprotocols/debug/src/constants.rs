use strata_l1_txfmt::{SubprotocolId, TxType};

/// The unique identifier for the Debug subprotocol within the Anchor State Machine.
///
/// This constant is set to a high value (255) to avoid conflicts with production subprotocols.
/// The debug subprotocol is only available when compiled with the "debug" feature flag.
pub const DEBUG_SUBPROTOCOL_ID: SubprotocolId = 255;

/// Transaction type for OL message injection.
///
/// This transaction type allows injection of arbitrary log messages into the ASM,
/// simulating logs that would normally come from the bridge subprotocol.
/// e.g. Deposit (Lock funds in n/n multisig)
pub const OLMSG_TX_TYPE: TxType = 1;

/// Transaction type for fake withdrawal creation.
///
/// This transaction type allows creation of withdrawal commands that are sent to
/// the bridge subprotocol, simulating withdrawals from the orchestration layer.
/// e.g. Withdrawal (Operator fronts payment)
pub const FAKEWITHDRAW_TX_TYPE: TxType = 2;

/// Transaction type for deposit unlock (future enhancement).
///
/// This transaction type will emit deposit unlock authorization signals
/// when the bridge interface changes to support direct deposit unlocks.
/// e.g. DepositUnlock (Unlock funds from n/n multisig and send to operator)
/// TODO: We need design and test logic around this
pub const UNLOCKDEPOSIT_TX_TYPE: TxType = 3;
