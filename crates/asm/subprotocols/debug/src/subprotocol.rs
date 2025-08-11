//! Debug subprotocol implementation.
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{
    logging, AnchorState, AsmError, AsmLog, AsmLogEntry, InterprotoMsg, MsgRelayer, Subprotocol,
    SubprotocolId, TxInputRef,
};
use strata_asm_proto_bridge_v1::{BridgeIncomingMsg, WithdrawOutput};
use strata_msg_fmt::TypeId;

use crate::{
    constants::DEBUG_SUBPROTOCOL_ID,
    txs::{parse_debug_tx, ParsedDebugTx},
};

/// Debug subprotocol implementation.
///
/// This subprotocol provides testing capabilities by processing special
/// L1 transactions that inject mock data into the ASM.
pub struct DebugSubproto;

/// Auxiliary input type for the debug subprotocol.
///
/// The debug subprotocol doesn't require any auxiliary input.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct DebugAuxInput;

impl Subprotocol for DebugSubproto {
    const ID: SubprotocolId = DEBUG_SUBPROTOCOL_ID;

    type Msg = DebugIncomingMsg;
    type GenesisConfig = ();
    type State = ();
    type AuxInput = DebugAuxInput;

    fn init(_config: Self::GenesisConfig) -> Result<Self::State, AsmError> {
        logging::info!("Initializing debug subprotocol state");
        Ok(())
    }

    fn process_txs(
        _state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        _aux_inputs: &[Self::AuxInput],
        relayer: &mut impl MsgRelayer,
    ) {
        for tx_ref in txs {
            logging::debug!(
                tx_type = tx_ref.tag().tx_type(),
                "Processing debug transaction"
            );

            match parse_debug_tx(tx_ref) {
                Ok(parsed_tx) => {
                    if let Err(e) = process_parsed_debug_tx(parsed_tx, relayer) {
                        logging::warn!("Failed to process debug transaction: {}", e);
                    }
                }
                Err(e) => {
                    logging::warn!("Failed to parse debug transaction: {}", e);
                }
            }
        }
    }

    fn process_msgs(_state: &mut Self::State, msgs: &[Self::Msg]) {
        for msg in msgs {
            match msg {
                DebugIncomingMsg::TestMessage(content) => {
                    logging::info!("Received test message: {}", content);
                    // Just log the message for now
                }
            }
        }
    }
}

/// Messages that can be sent to the debug subprotocol.
#[derive(Debug, Clone)]
pub enum DebugIncomingMsg {
    /// A test message for debugging purposes.
    TestMessage(String),
}

/// Log type for OL message injection.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct OlMsgLog {
    pub type_id: u32,
    pub payload: Vec<u8>,
}

impl AsmLog for OlMsgLog {
    const TY: TypeId = 9901; // High value to avoid conflicts
}

/// Log type for deposit unlock.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct UnlockDepositLog {
    pub deposit_id: u64,
}

impl AsmLog for UnlockDepositLog {
    const TY: TypeId = 9902; // High value to avoid conflicts
}

/// Wrapper to send BridgeIncomingMsg as an InterprotoMsg
#[derive(Debug, Clone)]
struct BridgeMessageWrapper(BridgeIncomingMsg);

impl InterprotoMsg for BridgeMessageWrapper {
    fn id(&self) -> SubprotocolId {
        // Target the bridge subprotocol
        strata_asm_proto_bridge_v1::BRIDGE_V1_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn std::any::Any {
        &self.0
    }
}

/// Process a parsed debug transaction.
fn process_parsed_debug_tx(
    parsed_tx: ParsedDebugTx,
    relayer: &mut impl MsgRelayer,
) -> Result<(), AsmError> {
    match parsed_tx {
        ParsedDebugTx::OlMsg(info) => {
            logging::info!(
                type_id = info.type_id,
                payload_len = info.payload.len(),
                "Processing OL message injection"
            );

            // Create and emit the log
            let log = OlMsgLog {
                type_id: info.type_id,
                payload: info.payload,
            };

            let log_entry = AsmLogEntry::from_log(&log)?;
            relayer.emit_log(log_entry);

            logging::info!("Successfully emitted OL message log");
        }

        ParsedDebugTx::FakeWithdraw(info) => {
            logging::info!(amount = info.amt.to_sat(), "Processing fake withdrawal");

            // Create withdrawal output using the bridge's type
            let withdrawal_output = WithdrawOutput::new(info.dest, info.amt);

            // Wrap it in BridgeIncomingMsg
            let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdrawal_output);

            // Send to bridge subprotocol via wrapper
            let wrapper = BridgeMessageWrapper(bridge_msg);
            relayer.relay_msg(&wrapper);

            logging::info!("Successfully sent fake withdrawal to bridge");
        }

        ParsedDebugTx::UnlockDeposit(info) => {
            logging::info!(deposit_id = info.deposit_id, "Processing unlock deposit");

            // TODO: Clarify and implement proper deposit unlock mechanism
            // Currently just emitting a log, but this should be updated once
            // the bridge subprotocol's deposit unlock interface is finalized.
            // This might involve:
            // - Sending a specific message type to bridge
            // - Emitting a specific log format that bridge listens to
            // - Direct state manipulation through a different mechanism

            // For now, emit a log indicating the deposit unlock request
            let log = UnlockDepositLog {
                deposit_id: info.deposit_id,
            };

            let log_entry = AsmLogEntry::from_log(&log)?;
            relayer.emit_log(log_entry);

            logging::info!("Successfully emitted deposit unlock log");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_subprotocol_id() {
        assert_eq!(DebugSubproto::ID, DEBUG_SUBPROTOCOL_ID);
    }
}
