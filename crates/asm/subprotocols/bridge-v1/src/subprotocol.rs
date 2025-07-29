//! Bridge V1 Subprotocol Implementation
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef,
    logging::{error, info, warn},
};
use strata_asm_logs::NewExportEntry;
use strata_primitives::{buf::Buf32, l1::L1BlockId};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, WITHDRAWAL_TX_TYPE},
    msgs::BridgeIncomingMsg,
    state::{BridgeV1Config, BridgeV1State},
    txs::{
        deposit::parse::extract_deposit_info,
        withdrawal_fulfillment::parse::extract_withdrawal_info,
    },
};

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

    type GenesisConfig = BridgeV1Config;

    fn init(genesis_config: Self::GenesisConfig) -> Result<Self::State, AsmError> {
        Ok(BridgeV1State::new(&genesis_config))
    }

    /// Processes transactions for the Bridge V1 subprotocol and handles expired assignment
    /// reassignment.
    ///
    /// This function is the main transaction processing entry point that:
    /// 1. Processes each transaction based on its type:
    ///    - **Deposit transactions** (`DEPOSIT_TX_TYPE`): Validates and records Bitcoin deposits in
    ///      the bridge state, making them available for withdrawal assignment
    ///    - **Withdrawal transactions** (`WITHDRAWAL_TX_TYPE`): Validates and processes withdrawal
    ///      fulfillments, removing completed assignments from the state
    /// 2. After processing all transactions, reassigns any expired assignments to new operators
    ///    that haven't previously failed on the same withdrawal
    ///
    /// # Parameters
    ///
    /// - `state` - Mutable reference to the bridge state
    /// - `txs` - Array of transaction input references to process
    /// - `anchor_pre` - Current anchor state containing chain view and block information
    /// - `_aux_inputs` - Auxiliary inputs (unused in Bridge V1)
    /// - `relayer` - Message relayer for emitting logs and events
    ///
    /// # Transaction Types Processed
    ///
    /// - **Deposit transactions**: Bitcoin transactions that lock funds in the bridge's multisig,
    ///   creating new deposit entries that can be assigned for withdrawal
    /// - **Withdrawal transactions**: Bitcoin transactions that fulfill assigned withdrawals,
    ///   completing the bridge process and removing assignments
    ///
    /// # Post-Processing
    ///
    /// After all transactions are processed, the function identifies and reassigns expired
    /// assignments using the current Bitcoin block height from the anchor state. This ensures
    /// that failed operators don't block withdrawals indefinitely.
    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
        _aux_inputs: &[Self::AuxInput],
        relayer: &mut impl MsgRelayer,
    ) {
        // Process each transaction based on its type
        for tx in txs {
            match tx.tag().tx_type() {
                DEPOSIT_TX_TYPE => Self::process_deposit_tx(state, tx),
                WITHDRAWAL_TX_TYPE => Self::process_withdrawal_tx(state, tx, relayer),
                _ => continue, // Ignore unsupported transaction types
            }
        }

        // After processing all transactions, reassign expired assignments
        let current_block = &anchor_pre.chain_view.pow_state.last_verified_block;

        match state.reassign_expired_assignments(current_block) {
            Ok(reassigned_deposits) => {
                if !reassigned_deposits.is_empty() {
                    info!(
                        count = reassigned_deposits.len(),
                        deposits = ?reassigned_deposits,
                        "Successfully reassigned expired assignments"
                    );
                }
            }
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to reassign expired assignments"
                );
            }
        }
    }

    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg]) {
        for msg in msgs {
            match msg {
                BridgeIncomingMsg::ProcessWithdrawal(withdrawal_cmd) => {
                    // TODO: Pass actual L1BlockId instead of placeholder
                    let placeholder_id = L1BlockId::from(Buf32::zero());
                    let current_block_height = 0;
                    if let Err(e) = state.create_withdrawal_assignment(
                        withdrawal_cmd,
                        &placeholder_id,
                        current_block_height,
                    ) {
                        error!(
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
    fn process_deposit_tx(state: &mut BridgeV1State, tx: &TxInputRef<'_>) {
        let deposit_info = match extract_deposit_info(tx) {
            Ok(info) => info,
            Err(e) => {
                warn!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to extract deposit information from transaction"
                );
                return;
            }
        };

        match state.process_deposit_tx(tx.tx(), &deposit_info) {
            Ok(_) => {}
            Err(e) => {
                error!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to process deposit"
                );
                return;
            }
        };

        info!(
            tx_id = %tx.tx().compute_txid(),
            idx = %deposit_info.deposit_idx,
            amount = %deposit_info.amt,
            "Successfully processed deposit"
        );
    }

    /// Processes a withdrawal fulfillment transaction with error logging.
    fn process_withdrawal_tx(
        state: &mut BridgeV1State,
        tx: &TxInputRef<'_>,
        relayer: &mut impl MsgRelayer,
    ) {
        let withdrawal_info = match extract_withdrawal_info(tx) {
            Ok(info) => info,
            Err(e) => {
                warn!(
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
                    warn!(
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

        info!(
            tx_id = %tx.tx().compute_txid(),
            deposit_idx = withdrawal_info.deposit_idx,
            operator_idx = withdrawal_info.operator_idx,
            amount = %withdrawal_info.withdrawal_amount,
            "Successfully processed withdrawal"
        );
    }
}
