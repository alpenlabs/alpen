//! BridgeV1 Subprotocol
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{
    AnchorState, AsmError, AuxInputCollector, BRIDGE_SUBPROTOCOL_ID, MsgRelayer, NullMsg,
    Subprotocol, SubprotocolId, TxInputRef,
};

/// BridgeV1 state.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1State {
    // TODO: Add bridge-specific state fields when implementing
}

/// Genesis configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, Default, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1GenesisConfig {
    // TODO: Add bridge-specific genesis parameters when implementing
}

/// BridgeV1 subprotocol impl.
#[derive(Copy, Clone, Debug)]
pub struct BridgeV1Subproto;

impl Subprotocol for BridgeV1Subproto {
    const ID: SubprotocolId = BRIDGE_SUBPROTOCOL_ID;

    type State = BridgeV1State;

    type Msg = NullMsg<BRIDGE_SUBPROTOCOL_ID>;

    type AuxInput = ();

    type GenesisConfig = BridgeV1GenesisConfig;

    fn init(_genesis_config_data: Option<&[u8]>) -> std::result::Result<Self::State, AsmError> {
        // For now, always return default state regardless of genesis config
        Ok(BridgeV1State {})
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
