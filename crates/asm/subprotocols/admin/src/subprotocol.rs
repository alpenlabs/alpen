//! Administration Subprotocol Implementation
//!
//! This module contains the core administration subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM) for managing protocol governance and updates.

use strata_asm_common::{
    MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInputRef, VerifiedAuxData,
};
use strata_asm_params::AdministrationInitConfig;
use strata_asm_txs_admin::{constants::ADMINISTRATION_SUBPROTOCOL_ID, parser::parse_tx};
use strata_primitives::L1BlockCommitment;

use crate::{
    handler::{handle_action, handle_pending_updates},
    state::AdministrationSubprotoState,
};

/// Administration subprotocol implementation.
///
/// This struct implements the [`Subprotocol`] trait to integrate administration functionality
/// with the ASM. It handles multisig governance actions, protocol parameter updates, and
/// operator set management through a queued execution system.
#[derive(Debug)]
pub struct AdministrationSubprotocol;

impl Subprotocol for AdministrationSubprotocol {
    const ID: SubprotocolId = ADMINISTRATION_SUBPROTOCOL_ID;

    type InitConfig = AdministrationInitConfig;

    type State = AdministrationSubprotoState;

    type Msg = NullMsg<ADMINISTRATION_SUBPROTOCOL_ID>;

    fn init(config: &Self::InitConfig) -> AdministrationSubprotoState {
        AdministrationSubprotoState::new(config)
    }

    /// Processes transactions for the Administration subprotocol and executes pending updates.
    ///
    /// The function follows a two-phase approach:
    /// 1. **Pre-processing**: Executes all queued updates that are ready for activation
    /// 2. **Transaction processing**: Handles incoming multisig actions
    fn process_txs(
        state: &mut AdministrationSubprotoState,
        txs: &[TxInputRef<'_>],
        l1ref: &L1BlockCommitment,
        _verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
    ) {
        let current_height = l1ref.height();

        // Phase 1: Execute any pending updates that have reached their activation height
        handle_pending_updates(state, relayer, current_height);

        // Phase 2: Process incoming administration transactions
        for tx in txs {
            if let Ok(signed_payload) = parse_tx(tx) {
                let _ = handle_action(state, signed_payload, current_height, relayer);
            }
            // Transaction parsing failures are silently ignored to maintain system resilience
        }
    }

    /// Processes incoming administration messages.
    ///
    /// Currently, the Administration subprotocol uses `NullMsg` and does not process
    /// any incoming messages. All administration actions are handled through transactions
    /// in the `process_txs` method.
    fn process_msgs(
        _state: &mut AdministrationSubprotoState,
        _msgs: &[Self::Msg],
        _l1ref: &L1BlockCommitment,
    ) {
    }
}
