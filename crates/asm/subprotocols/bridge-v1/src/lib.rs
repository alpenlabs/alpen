//! BridgeV1 Subprotocol
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{NullMsg, Subprotocol, SubprotocolId};

/// The unique identifier for the BridgeV1 subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the CoreASM logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const BRIDGE_V1_SUBPROTOCOL_ID: SubprotocolId = 2;

/// BridgeV1 state.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1State {
    // TODO
}

/// BridgeV1 subprotocol impl.
#[derive(Copy, Clone, Debug)]
pub struct BridgeV1Subproto;

impl Subprotocol for BridgeV1Subproto {
    const ID: SubprotocolId = BRIDGE_V1_SUBPROTOCOL_ID;

    type State = BridgeV1State;

    type Msg = NullMsg<BRIDGE_V1_SUBPROTOCOL_ID>;

    type AuxInput = ();

    fn init() -> Self::State {
        todo!()
    }

    fn pre_process_txs(
        _state: &Self::State,
        _txs: &[strata_asm_common::TxInput<'_>],
        _collector: &mut impl strata_asm_common::AuxInputCollector,
    ) {
        todo!()
    }

    fn process_txs(
        _state: &mut Self::State,
        _txs: &[strata_asm_common::TxInput<'_>],
        _aux_inputs: &[Self::AuxInput],
        _relayer: &mut impl strata_asm_common::MsgRelayer,
    ) {
        todo!()
    }

    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg]) {
        todo!()
    }
}
