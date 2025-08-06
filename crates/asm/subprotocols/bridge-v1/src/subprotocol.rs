//! Bridge V1 Subprotocol Implementation
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use strata_asm_common::{
    AnchorState, AsmError, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef,
    logging::{error, info},
};
use strata_primitives::{
    buf::Buf32,
    l1::{L1BlockCommitment, L1BlockId},
};

use crate::{
    constants::BRIDGE_V1_SUBPROTOCOL_ID,
    msgs::BridgeIncomingMsg,
    state::{BridgeV1Config, BridgeV1State},
    txs::{handle_parsed_tx, parse_tx},
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
        anchor_pre: &AnchorState,
        _aux_inputs: &[Self::AuxInput],
        relayer: &mut impl MsgRelayer,
    ) {
        // Process each transaction
        for tx in txs {
            // Parse transaction to extract structured data (deposit/withdrawal info)
            // then handle the parsed transaction to update state and emit events
            match parse_tx(tx).and_then(|parsed_tx| handle_parsed_tx(state, parsed_tx, relayer)) {
                // `tx_id` is computed inside macro, because logging is compiled to noop in ZkVM
                Ok(()) => info!(tx_id = %tx.tx().compute_txid(), "Successfully processed tx"),
                Err(e) => {
                    error!(tx_id = %tx.tx().compute_txid(), error = %e, "Failed to process tx")
                }
            }
        }

        // After processing all transactions, reassign expired assignments
        let current_block = &anchor_pre.chain_view.pow_state.last_verified_block;
        match state.reassign_expired_assignments(current_block) {
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
    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg]) {
        for msg in msgs {
            match msg {
                BridgeIncomingMsg::DispatchWithdrawal(withdrawal_cmd) => {
                    // TODO: Pass actual L1BlockId instead of placeholder
                    let l1blk = L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero()));
                    if let Err(e) = state.create_withdrawal_assignment(withdrawal_cmd, &l1blk) {
                        // PANIC: Withdrawal assignment failure indicates catastrophic system
                        // compromise.
                        panic!("Failed to create withdrawal assignment: {e}",);
                    }
                }
            }
        }
    }
}
