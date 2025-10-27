//! # CoreASM Subprotocol
//!
//! This module implements the "CoreASM" subprotocol, responsible for
//! on-chain verification and anchoring of zk-SNARK checkpoint proofs.
//!
//! ## Overview
//!
//! The Core subprotocol is the central component of the Anchor State Machine (ASM)
//! that manages checkpoint verification and state transitions. It ensures that:
//!
//! - Each zk-SNARK proof of a new checkpoint is correctly verified
//! - State transitions follow the protocol rules
//! - Withdrawal messages are properly forwarded to the Bridge subprotocol
//! - Administrative keys (sequencer, verifying key) can be safely updated
//!
//! ## Key Components
//!
//! - **Checkpoint Verification**: Validates zk-SNARK proofs and state transitions
//! - **Message Handling**: Processes inter-subprotocol communications
//! - **State Management**: Maintains the latest verified checkpoint state
//! - **Withdrawal Processing**: Extracts and forwards L2→L1 withdrawal messages
//!
//! ## Transaction Types
//!
//! The Core subprotocol processes three types of transactions:
//!
//! 1. **OL STF Checkpoint** (`OL_STF_CHECKPOINT_TX_TYPE`): Contains signed checkpoint proofs
//! 2. **Forced Inclusion** (`FORCED_INCLUSION_TX_TYPE`): TBD
//! 3. **EE Upgrade** (`EE_UPGRADE_TX_TYPE`): TBD
//!
//! ## Security Considerations
//!
//! - All public parameters are constructed from trusted state, not sequencer input
//! - Signature verification prevents unauthorized checkpoint submissions
//! - State validation ensures proper progression of epochs and block heights
//! - Rolling hash verification prevents L1→L2 message manipulation
// TODO rename "core" to "checkpoint" all over the place

// Module declarations
mod constants;
mod error;
mod handlers;
mod messages;
mod parsing;
mod types;
mod verification;

// Public re-exports
use constants::CORE_SUBPROTOCOL_ID;
pub use error::*;
#[cfg(feature = "preprocess")]
use strata_asm_common::AuxRequests;
use strata_asm_common::{
    AnchorState, AsmError, AuxInput, MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInputRef,
    logging,
};
use strata_checkpoint_types::EpochSummary;
use strata_primitives::{buf::Buf32, l2::L2BlockCommitment};
pub use types::{CoreGenesisConfig, CoreOLState};

use crate::{constants::OL_STF_CHECKPOINT_TX_TYPE, handlers::handle_checkpoint_transaction};

/// OL Core subprotocol.
///
/// The OL Core subprotocol ensures that each zk‐SNARK proof of a new checkpoint
/// is correctly verified against the last known checkpoint state anchored on L1.
/// It manages the verifying key, tracks the latest verified checkpoint, and
/// enforces administrative controls over batch producer and consensus manager keys.
// TODO move to its own module, like in the other crate
#[derive(Copy, Clone, Debug)]
pub struct OLCoreSubproto;

impl Subprotocol for OLCoreSubproto {
    const ID: SubprotocolId = CORE_SUBPROTOCOL_ID;

    type State = CoreOLState;

    type Params = CoreGenesisConfig;

    // [PLACE_HOLDER]
    // TODO: Define the message type for inter-subprotocol communication
    // type of msg that we receive from other subprotocols
    type Msg = NullMsg<CORE_SUBPROTOCOL_ID>;

    fn init(params: &Self::Params) -> std::result::Result<Self::State, AsmError> {
        // Construct genesis EpochSummary from the complete L1 block information
        // At genesis time, we have the complete L1 block commitment (ID + height)
        let genesis_epoch_summary = EpochSummary::new(
            0,                         // epoch: genesis is epoch 0
            L2BlockCommitment::null(), // terminal: no L2 blocks yet
            L2BlockCommitment::null(), // prev_terminal: no previous epoch
            params.genesis_l1_block,   // new_l1: complete L1 block commitment
            Buf32::zero(),             // final_state: genesis state (zero)
        );

        // Initialize the Core subprotocol state from genesis configuration
        Ok(CoreOLState {
            checkpoint_predicate: params.checkpoint_predicate.clone(),
            verified_checkpoint: genesis_epoch_summary,
            last_checkpoint_ref: *params.genesis_l1_block.blkid(),
            sequencer_pubkey: params.sequencer_pubkey,
        })
    }

    #[cfg(feature = "preprocess")]
    fn pre_process_txs(
        _state: &Self::State,
        _txs: &[TxInputRef<'_>],
        _requests: &mut AuxRequests,
        _anchor_pre: &AnchorState,
        _params: &Self::Params,
    ) {
        // [PLACE_HOLDER]
        // TODO: Waiting for auxiliary input to be defined
        // it's also dependent on the history_mmr and public_params of zk proof
    }

    // Transactions come from L1 and can be submitted by anyone, so we handle tx processing failures
    // gracefully. Invalid transactions are logged and ignored rather than causing panics or
    // halting processing.
    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
        aux_resolver: &AuxInput,
        relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
    ) {
        for tx in txs {
            let result = match tx.tag().tx_type() {
                OL_STF_CHECKPOINT_TX_TYPE => {
                    handle_checkpoint_transaction(state, tx, relayer, anchor_pre, aux_resolver)
                }
                // [PLACE_HOLDER] Add other transaction types related to vk upgrade, etc.
                _ => Err(CoreError::TxParsingError("unsupported tx type".to_string())),
            };

            // Log transaction processing errors using zkVM-compatible logging.
            // We can't propagate errors to upper layers when transaction processing fails because
            // invalidating and rejecting transactions is normal and expected behavior. We don't
            // want to halt the entire block processing because of a single invalid transaction.
            if let Err(e) = result {
                let txid = tx.tx().compute_txid();
                logging::warn!("Error processing transaction (txid: {txid:?}): {e:?}");
            }
        }
    }

    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg], _params: &Self::Params) {
        // [PLACE_HOLDER]
        // TODO: Implement message processing from upgrade subprotocol messages
        // to update verifying key and sequencer key.
    }
}
