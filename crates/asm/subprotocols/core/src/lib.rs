//! This module implements the “CoreASM” subprotocol, responsible for
//! on-chain verification and anchoring of zk-SNARK checkpoint proofs.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInput};
use strata_primitives::{batch::EpochSummary, buf::Buf32, l1::L1BlockId};
use zkaleido::VerifyingKey;

/// The unique identifier for the CoreASM subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the CoreASM logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const CORE_SUBPROTOCOL_ID: SubprotocolId = 1;

/// OL Core state.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CoreOLState {
    /// The zk‐SNARK verifying key used to verify each new checkpoint proof
    /// that has been posted on Bitcoin.
    checkpoint_vk: VerifyingKey,

    /// Summary of the last checkpoint that was successfully verified.
    /// New proofs are checked against this epoch summary.
    verified_checkpoint: EpochSummary,

    /// The L1 block ID up to which the `verified_checkpoint` covers.
    last_checkpoint_ref: L1BlockId,

    /// Public key of the sequencer authorized to submit checkpoint proofs.
    sequencer_pubkey: Buf32,
}

/// OL Core subprotocol.
///
/// The OL Core subprotocol ensures that each zk‐SNARK proof of a new checkpoint
/// is correctly verified against the last known checkpoint state anchored on L1.
/// It manages the verifying key, tracks the latest verified checkpoint, and
/// enforces administrative controls over batch producer and consensus manager keys.
#[derive(Copy, Clone, Debug)]
pub struct OLCoreSubproto;

impl Subprotocol for OLCoreSubproto {
    const ID: SubprotocolId = CORE_SUBPROTOCOL_ID;

    type State = CoreOLState;

    type Msg = NullMsg<CORE_SUBPROTOCOL_ID>;

    fn init() -> Self::State {
        todo!()
    }

    fn process_txs(_state: &mut Self::State, _txs: &[TxInput<'_>], _relayer: &mut impl MsgRelayer) {
        todo!()
    }

    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg]) {
        todo!()
    }
}
