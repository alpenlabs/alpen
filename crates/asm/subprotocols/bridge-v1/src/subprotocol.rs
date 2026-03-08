//! Bridge V1 Subprotocol Implementation
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use strata_asm_bridge_msgs::BridgeIncomingMsg;
use strata_asm_common::{
    AuxRequestCollector, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef, VerifiedAuxData,
    logging::{error, info},
};
use strata_asm_params::BridgeV1InitConfig;
use strata_asm_txs_bridge_v1::{BRIDGE_V1_SUBPROTOCOL_ID, parser::parse_tx};
use strata_primitives::l1::L1BlockCommitment;

use crate::{
    errors::BridgeSubprotocolError,
    handler::{handle_parsed_tx, preprocess_parsed_tx},
    state::BridgeV1State,
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

    type InitConfig = BridgeV1InitConfig;

    type Msg = BridgeIncomingMsg;

    fn init(config: &Self::InitConfig) -> Self::State {
        BridgeV1State::new(config)
    }

    /// Pre-processes transactions to collect auxiliary data requests.
    ///
    /// This function runs before the main transaction processing to identify and request
    /// any auxiliary data needed for verification.
    fn pre_process_txs(
        state: &Self::State,
        txs: &[TxInputRef<'_>],
        collector: &mut AuxRequestCollector,
    ) {
        // Pre-Process each transaction
        for tx in txs {
            // Parse transaction to extract structured data, then handle the preprocess transaction
            // to get the auxiliary requests
            match parse_tx(tx) {
                Ok(parsed_tx) => {
                    preprocess_parsed_tx(parsed_tx, state, collector);
                }
                Err(e) => {
                    error!(
                        txid = %tx.tx().compute_txid(),
                        error = %e,
                        "Failed to pre-process tx"
                    )
                }
            }
        }
    }

    /// Processes transactions and reassigns expired assignments.
    ///
    /// The function follows a two-phase approach:
    /// 1. **Transaction processing**: Handles incoming bridge transactions
    /// 2. **Post-processing**: Reassigns any expired assignments to new operators
    ///
    /// # Panics
    ///
    /// **CRITICAL**: This function panics if expired assignment reassignment fails, as this
    /// indicates a violation of the bridge's 1/N honesty assumption. The bridge protocol assumes at
    /// least one honest operator remains active to fulfill withdrawals. Failure to reassign
    /// expired assignments means no honest operators are available, representing an
    /// unrecoverable protocol breach that poses significant risk of fund loss.
    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        l1ref: &L1BlockCommitment,
        verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
    ) {
        // Process each transaction
        for tx in txs {
            // Parse transaction to extract structured data (deposit/withdrawal info)
            // then handle the parsed transaction to update state and emit events
            match parse_tx(tx)
                .map_err(BridgeSubprotocolError::from)
                .and_then(|parsed_tx| {
                    handle_parsed_tx(state, parsed_tx, verified_aux_data, relayer)
                }) {
                // `tx_id` is computed inside macro, because logging is compiled to noop in ZkVM
                Ok(()) => info!(tx_id = %tx.tx().compute_txid(), "Successfully processed tx"),
                Err(e) => {
                    error!(tx_id = %tx.tx().compute_txid(), error = %e, "Failed to process tx")
                }
            }
        }

        // After processing all transactions, reassign expired assignments
        match state.reassign_expired_assignments(l1ref) {
            Ok(reassigned_deposits) => {
                info!(
                    count = reassigned_deposits.len(),
                    deposits = ?reassigned_deposits,
                    "Successfully reassigned expired assignments"
                );
            }
            Err(e) => {
                // PANIC: Failure to reassign expired assignments indicates a violation of the
                // bridge's fundamental 1/N honesty assumption. This means no operators remain
                // available to fulfill withdrawals, representing an unrecoverable protocol breach
                // that poses significant risk of fund loss.
                panic!("Failed to reassign expired assignments {e}");
            }
        }
    }

    /// Processes incoming bridge messages
    ///
    /// This function handles messages sent to the bridge subprotocol. Currently processes:
    ///
    /// - **`DispatchWithdrawal`**: Creates withdrawal assignments by selecting available operators
    ///   to fulfill pending withdrawals. The assignment process ensures proper operator selection
    ///   based on availability, stake, and previous failure history.
    ///
    /// # Panics
    ///
    /// **CRITICAL**: This function panics if withdrawal assignment creation fails, as this
    /// indicates one of two catastrophic system failures:
    ///
    /// 1. **1/N Honest Assumption Violated**: No honest operators remain active, breaking the
    ///    fundamental security assumption of the bridge protocol
    /// 2. **Peg Mechanism Failure**: The bridge's peg to Bitcoin has been compromised, potentially
    ///    due to operator collusion or critical implementation bugs
    ///
    /// Both conditions represent unrecoverable protocol violations where continued operation
    /// poses significant risk of fund loss.
    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], l1ref: &L1BlockCommitment) {
        for msg in msgs {
            match msg {
                BridgeIncomingMsg::DispatchWithdrawal {
                    output,
                    selected_operator,
                } => {
                    if let Err(e) =
                        state.create_withdrawal_assignment(output, *selected_operator, l1ref)
                    {
                        // PANIC: Withdrawal assignment failure indicates catastrophic system
                        // compromise.
                        panic!("Failed to create withdrawal assignment: {e}",);
                    }
                }
            }
        }
    }
}
