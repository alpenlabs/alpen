//! Bridge V1 Subprotocol Implementation
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, AuxInputCollector, MsgRelayer, Subprotocol, SubprotocolId,
    TxInputRef,
};
use strata_asm_logs::NewExportEntry;
use strata_primitives::buf::Buf32;

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, WITHDRAWAL_TX_TYPE},
    msgs::BridgeIncomingMsg,
    state::BridgeV1State,
    txs::{deposit::extract_deposit_info, withdrawal::extract_withdrawal_info},
};

/// Genesis configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1GenesisConfig {
    /// Initial operator table for the bridge
    pub operators: crate::state::OperatorTable,
    /// Expected deposit denomination for validation
    pub denomination: strata_primitives::l1::BitcoinAmount,
    /// Duration in blocks for assignment execution deadlines
    pub deadline_duration: u64,
}

/// Bridge V1 subprotocol implementation.
///
/// This struct implements the [`Subprotocol`] trait to integrate the bridge functionality
/// with the ASM. It handles Bitcoin deposit processing, operator management, and withdrawal
/// coordination.
#[derive(Copy, Clone, Debug)]
pub struct BridgeV1Subproto;

impl Subprotocol for BridgeV1Subproto {
    const ID: SubprotocolId = BRIDGE_V1_SUBPROTOCOL_ID;

    type State = BridgeV1State;

    type Msg = BridgeIncomingMsg;

    type AuxInput = ();

    type GenesisConfig = BridgeV1GenesisConfig;

    fn init(genesis_config: Self::GenesisConfig) -> std::result::Result<Self::State, AsmError> {
        Ok(BridgeV1State::new(genesis_config.operators, genesis_config.denomination, genesis_config.deadline_duration))
    }

    fn pre_process_txs(
        _state: &Self::State,
        _txs: &[TxInputRef<'_>],
        _collector: &mut impl AuxInputCollector,
        _anchor_pre: &AnchorState,
    ) {
        // No auxiliary input needed for bridge subprotocol processing
    }

    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        _aux_inputs: &[Self::AuxInput],
        relayer: &mut impl MsgRelayer,
    ) {
        for tx in txs {
            match tx.tag().tx_type() {
                DEPOSIT_TX_TYPE => Self::process_deposit_tx(state, tx),
                WITHDRAWAL_TX_TYPE => Self::process_withdrawal_tx(state, tx, relayer),
                _ => continue, // Ignore unsupported transaction types
            }
        }
    }

    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg]) {
        for msg in msgs {
            match msg {
                BridgeIncomingMsg::ProcessWithdrawal(withdrawal_cmd) => {
                    // TODO: Pass actual L1BlockHash instead of placeholder
                    let placeholder_hash = Buf32::zero();
                    let current_block_height = 0;
                    if let Err(e) = state.create_withdrawal_assignment(
                        withdrawal_cmd,
                        &placeholder_hash,
                        current_block_height,
                    ) {
                        tracing::error!(
                            error = %e,
                            "Failed to create withdrawal assignment"
                        );
                    }
                }
            }
        }
    }
}

impl BridgeV1Subproto {
    /// Processes a deposit transaction with error logging.
    fn process_deposit_tx(state: &mut BridgeV1State, tx: &strata_asm_common::TxInputRef<'_>) {
        let deposit_info = match extract_deposit_info(tx) {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to extract deposit information from transaction"
                );
                return;
            }
        };

        let deposit_idx = match state.process_deposit_tx(tx.tx(), &deposit_info) {
            Ok(idx) => idx,
            Err(e) => {
                tracing::error!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to process deposit"
                );
                return;
            }
        };

        tracing::info!(
            tx_id = %tx.tx().compute_txid(),
            deposit_idx = deposit_idx,
            amount = %deposit_info.amt,
            "Successfully processed deposit"
        );
    }

    /// Processes a withdrawal fulfillment transaction with error logging.
    fn process_withdrawal_tx(
        state: &mut BridgeV1State,
        tx: &strata_asm_common::TxInputRef<'_>,
        relayer: &mut impl strata_asm_common::MsgRelayer,
    ) {
        let withdrawal_info = match extract_withdrawal_info(tx) {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to extract withdrawal information from transaction"
                );
                return;
            }
        };

        let withdrawal_processed_info =
            match state.process_withdrawal_fulfillment_tx(tx.tx(), &withdrawal_info) {
                Ok(assignment) => assignment,
                Err(e) => {
                    tracing::warn!(
                        tx_id = %tx.tx().compute_txid(),
                        deposit_idx = withdrawal_info.deposit_idx,
                        operator_idx = withdrawal_info.operator_idx,
                        error = %e,
                        "Withdrawal validation failed"
                    );
                    return;
                }
            };

        // FIXME: This is a placeholder for the actual container ID logic.
        let container_id = 0; // Replace with actual logic to determine container ID
        let withdrawal_processed_log =
            NewExportEntry::new(container_id, withdrawal_processed_info.to_export_entry());
        relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:"));

        tracing::info!(
            tx_id = %tx.tx().compute_txid(),
            deposit_idx = withdrawal_info.deposit_idx,
            operator_idx = withdrawal_info.operator_idx,
            amount = %withdrawal_info.withdrawal_amount,
            "Successfully processed withdrawal"
        );
    }
}
