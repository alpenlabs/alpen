//! Debug subprotocol implementation.
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, MsgRelayer, NullMsg, Subprotocol, SubprotocolId,
    TxInputRef, logging,
};
use strata_asm_logs::AsmLogType;
use strata_asm_proto_bridge_v1::BridgeIncomingMsg;

use crate::{
    constants::DEBUG_SUBPROTOCOL_ID,
    txs::{ParsedDebugTx, parse_debug_tx},
};

/// Helper function to create an AsmLogEntry from any AsmLogType variant.
///
/// This provides a cleaner interface than pattern matching on each variant.
fn create_asm_log_entry(asm_log_type: &AsmLogType) -> Result<AsmLogEntry, AsmError> {
    match asm_log_type {
        AsmLogType::AsmStfUpdate(log) => AsmLogEntry::from_log(log),
        AsmLogType::CheckpointUpdate(log) => AsmLogEntry::from_log(log),
        AsmLogType::DepositLog(log) => AsmLogEntry::from_log(log),
        AsmLogType::NewExportEntry(log) => AsmLogEntry::from_log(log),
        AsmLogType::ForcedInclusionData(log) => AsmLogEntry::from_log(log),
    }
}

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
    type AuxInput = ();

    fn init(_config: &Self::Params) -> Result<Self::State, AsmError> {
        logging::info!("Initializing debug subprotocol state");
        Ok(())
    }

    fn process_txs(
        _state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        _aux_inputs: &Self::AuxInput,
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
        ParsedDebugTx::FakeAsmLog(asm_log_type) => {
            logging::info!("Processing ASM log injection");

            // Create log entry from the ASM log type using helper function
            let log_entry = create_asm_log_entry(&asm_log_type)?;

            relayer.emit_log(log_entry);
            logging::info!("Successfully emitted ASM log");
        }

        ParsedDebugTx::FakeWithdrawIntent(withdraw_output) => {
            logging::info!(
                amount = withdraw_output.amt.to_sat(),
                "Processing fake withdrawal"
            );

            // Wrap it in [`BridgeIncomingMsg`]
            let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);

            // Send to bridge subprotocol
            relayer.relay_msg(&bridge_msg);

            logging::info!("Successfully sent fake withdrawal to bridge");
        }
    }

    Ok(())
}
