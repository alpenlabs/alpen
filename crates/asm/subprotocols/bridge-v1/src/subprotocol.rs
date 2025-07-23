//! Bridge V1 Subprotocol Implementation
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{
    AnchorState, AsmError, AuxInputCollector, MsgRelayer, NullMsg, Subprotocol, SubprotocolId,
    TxInputRef,
};

use crate::state::BridgeV1State;

/// The unique identifier for the Bridge V1 subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the Bridge V1 logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const BRIDGE_V1_SUBPROTOCOL_ID: SubprotocolId = 2;

/// Genesis configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, Default, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1GenesisConfig {
    // TODO: Add bridge-specific genesis parameters when implementing
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

    type Msg = NullMsg<BRIDGE_V1_SUBPROTOCOL_ID>;

    type AuxInput = ();

    type GenesisConfig = BridgeV1GenesisConfig;

    fn init(_genesis_config: Self::GenesisConfig) -> std::result::Result<Self::State, AsmError> {
        // For now, always return default state regardless of genesis config
        Ok(BridgeV1State::default())
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
        _state: &mut Self::State,
        _txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        _aux_inputs: &[Self::AuxInput],
        _relayer: &mut impl MsgRelayer,
    ) {
        // TODO: Implement bridge transaction processing
    }

    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg]) {
        // TODO: Implement bridge message processing
    }
}
