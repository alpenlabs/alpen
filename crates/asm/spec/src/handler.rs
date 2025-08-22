//! Subprotocol handler.

use strata_asm_common::{
    AnchorState, AuxInputCollector, InterprotoMsg, MsgRelayer, SectionState, SubprotoHandler,
    Subprotocol, SubprotocolId, TxInputRef,
};

pub(crate) struct HandlerImpl<S: Subprotocol> {
    state: S::State,
    interproto_msg_buf: Vec<S::Msg>,
    aux_inputs: Vec<S::AuxInput>,
}

impl<S: Subprotocol + 'static> HandlerImpl<S> {
    pub(crate) fn new(state: S::State, aux_inputs: Vec<S::AuxInput>) -> Self {
        Self {
            state,
            aux_inputs,
            interproto_msg_buf: Vec::new(),
        }
    }
}

impl<S: Subprotocol> SubprotoHandler for HandlerImpl<S> {
    fn id(&self) -> SubprotocolId {
        S::ID
    }

    fn accept_msg(&mut self, msg: &dyn InterprotoMsg) {
        let m = msg
            .as_dyn_any()
            .downcast_ref::<S::Msg>()
            .expect("asm: incorrect interproto msg type");
        self.interproto_msg_buf.push(m.clone());
    }

    fn pre_process_txs(
        &mut self,
        txs: &[TxInputRef<'_>],
        collector: &mut dyn AuxInputCollector,
        anchor_pre: &AnchorState,
    ) {
        S::pre_process_txs(&self.state, txs, collector, anchor_pre);
    }

    fn process_txs(
        &mut self,
        txs: &[TxInputRef<'_>],
        relayer: &mut dyn MsgRelayer,
        anchor_pre: &AnchorState,
    ) {
        S::process_txs(&mut self.state, txs, anchor_pre, &self.aux_inputs, relayer);
    }

    fn process_buffered_msgs(&mut self) {
        S::process_msgs(&mut self.state, &self.interproto_msg_buf)
    }

    fn to_section(&self) -> SectionState {
        SectionState::from_state::<S>(&self.state)
    }
}
