//! Debug subprotocol implementation.
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use strata_asm_bridge_msgs::BridgeIncomingMsg;
use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, MsgRelayer, NullMsg, Subprotocol, SubprotocolId,
    TxInputRef, VerifiedAuxData, logging,
};

use crate::{
    constants::DEBUG_SUBPROTOCOL_ID,
    txs::{ParsedDebugTx, parse_debug_tx},
};

/// Debug subprotocol implementation.
///
/// This subprotocol provides testing capabilities by processing special
/// L1 transactions that inject mock data into the ASM.
#[derive(Copy, Clone, Debug)]
pub struct DebugSubproto;

impl Subprotocol for DebugSubproto {
    const ID: SubprotocolId = DEBUG_SUBPROTOCOL_ID;

    type Msg = NullMsg<DEBUG_SUBPROTOCOL_ID>;
    type Params = ();
    type State = ();

    fn init(_config: &Self::Params) -> Result<Self::State, AsmError> {
        logging::info!("Initializing debug subprotocol state");
        Ok(())
    }

    fn process_txs(
        _state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        _verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
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

    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg], _params: &Self::Params) {
        // No messages to process for the debug subprotocol
    }
}

/// Process a parsed debug transaction.
fn process_parsed_debug_tx(
    parsed_tx: ParsedDebugTx,
    relayer: &mut impl MsgRelayer,
) -> Result<(), AsmError> {
    match parsed_tx {
        ParsedDebugTx::MockAsmLog(log_info) => {
            logging::info!("Processing ASM log injection");

            // Create log entry directly from raw bytes
            // The log_info contains the raw bytes that represent the log
            let log_entry = AsmLogEntry::from_raw(log_info.bytes);

            relayer.emit_log(log_entry);
            logging::info!("Successfully emitted ASM log");
        }

        ParsedDebugTx::MockWithdrawIntent(withdraw_output) => {
            logging::info!(
                amount = withdraw_output.amt.to_sat(),
                "Processing mock withdrawal"
            );

            // Wrap it in [`BridgeIncomingMsg`]
            let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);

            // Send to bridge subprotocol
            relayer.relay_msg(&bridge_msg);

            logging::info!("Successfully sent mock withdrawal intent to bridge");
        }
    }

    Ok(())
}
