//! Subprotocol handler.

use std::{any::Any, collections::BTreeMap};

use strata_asm_common::{
    AnchorState, AsmError, AsmLog, AuxInputCollector, AuxRequest, InterprotoMsg, MsgRelayer,
    SectionState, SubprotoHandler, Subprotocol, SubprotocolId, TxInputRef,
};

/// Wrapper around the common subprotocol interface that handles the common
/// buffering logic for interproto messages.
pub(crate) struct HandlerImpl<S: Subprotocol, R, C> {
    state: S::State,
    interproto_msg_buf: Vec<S::Msg>,
    aux_inputs: Vec<S::AuxInput>,

    _r: std::marker::PhantomData<R>,
    _c: std::marker::PhantomData<C>,
}

impl<S: Subprotocol + 'static, R: MsgRelayer + 'static, C: AuxInputCollector + 'static>
    HandlerImpl<S, R, C>
{
    pub(crate) fn new(
        state: S::State,
        aux_inputs: Vec<S::AuxInput>,
        interproto_msg_buf: Vec<S::Msg>,
    ) -> Self {
        Self {
            state,
            aux_inputs,
            interproto_msg_buf,
            _r: std::marker::PhantomData,
            _c: std::marker::PhantomData,
        }
    }
}

impl<S: Subprotocol, R: MsgRelayer, C: AuxInputCollector> SubprotoHandler for HandlerImpl<S, R, C> {
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
        let collector = collector
            .as_mut_any()
            .downcast_mut::<C>()
            .expect("asm: handler");
        S::pre_process_txs(&self.state, txs, collector, anchor_pre);
    }

    fn process_txs(
        &mut self,
        txs: &[TxInputRef<'_>],
        relayer: &mut dyn MsgRelayer,
        anchor_pre: &AnchorState,
    ) {
        let relayer = relayer
            .as_mut_any()
            .downcast_mut::<R>()
            .expect("asm: handler");
        S::process_txs(&mut self.state, txs, anchor_pre, &self.aux_inputs, relayer);
    }

    fn process_buffered_msgs(&mut self) {
        S::process_msgs(&mut self.state, &self.interproto_msg_buf)
    }

    fn to_section(&self) -> SectionState {
        SectionState::from_state::<S>(&self.state)
    }
}

/// Manages subproto handlers and relays messages between them.
pub(crate) struct SubprotoManager {
    handlers: BTreeMap<SubprotocolId, Box<dyn SubprotoHandler>>,
    logs: Vec<AsmLog>,
    aux_requests: Vec<AuxRequest>,
}

impl SubprotoManager {
    /// Inserts a subproto by creating a handler for it, wrapping a tstate.
    pub(crate) fn insert_subproto<S: Subprotocol>(
        &mut self,
        state: S::State,
        aux_inputs: Vec<S::AuxInput>,
    ) {
        let handler = HandlerImpl::<S, Self, Self>::new(state, aux_inputs, Vec::new());
        assert_eq!(
            handler.id(),
            S::ID,
            "asm: subproto handler impl ID doesn't match"
        );
        self.insert_handler(Box::new(handler));
    }

    /// Dispatches pre-processing to the appropriate handler.
    ///
    /// This method temporarily removes the handler from the internal map to satisfy
    /// Rust’s borrow rules, invokes its `pre_process_txs` implementation with
    /// `self` acting as the `AuxInputCollector`, and then reinserts the handler.
    pub(crate) fn invoke_pre_process_txs<S: Subprotocol>(
        &mut self,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
    ) {
        // We temporarily take the handler out of the map so we can call
        // `process_txs` with `self` as the relayer without violating the
        // borrow checker.
        let mut h = self
            .remove_handler(S::ID)
            .expect("asm: unloaded subprotocol");
        h.pre_process_txs(txs, self, anchor_pre);
        self.insert_handler(h);
    }

    /// Dispatches transaction processing to the appropriate handler.
    ///
    /// This default implementation temporarily removes the handler to satisfy
    /// borrow-checker constraints, invokes `process_txs` with `self` as the relayer,
    /// and then reinserts the handler.
    pub(crate) fn invoke_process_txs<S: Subprotocol>(
        &mut self,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
    ) {
        // We temporarily take the handler out of the map so we can call
        // `process_txs` with `self` as the relayer without violating the
        // borrow checker.
        let mut h = self
            .remove_handler(S::ID)
            .expect("asm: unloaded subprotocol");
        h.process_txs(txs, self, anchor_pre);
        self.insert_handler(h);
    }

    /// Dispatches buffered inter-protocol message processing to the handler.
    pub(crate) fn invoke_process_msgs<S: Subprotocol>(&mut self) {
        let h = self
            .get_handler_mut(S::ID)
            .expect("asm: unloaded subprotocol");
        h.process_buffered_msgs()
    }

    fn insert_handler(&mut self, handler: Box<dyn SubprotoHandler>) {
        use std::collections::btree_map::Entry;

        // We have to make sure we don't overwrite something there.
        let ent = self.handlers.entry(handler.id());
        if matches!(ent, Entry::Occupied(_)) {
            panic!("asm: tried to overwrite subproto {} entry", handler.id());
        }

        ent.or_insert(handler);
    }

    fn remove_handler(&mut self, id: SubprotocolId) -> Result<Box<dyn SubprotoHandler>, AsmError> {
        self.handlers
            .remove(&id)
            .ok_or(AsmError::InvalidSubprotocol(id))
    }

    #[allow(unused)]
    fn get_handler(&self, id: SubprotocolId) -> Result<&dyn SubprotoHandler, AsmError> {
        self.handlers
            .get(&id)
            .map(Box::as_ref)
            .ok_or(AsmError::InvalidSubprotocol(id))
    }

    fn get_handler_mut(
        &mut self,
        id: SubprotocolId,
    ) -> Result<&mut Box<dyn SubprotoHandler>, AsmError> {
        self.handlers
            .get_mut(&id)
            .ok_or(AsmError::InvalidSubprotocol(id))
    }

    /// Extracts the section state for a subprotocol.
    #[allow(unused)]
    pub(crate) fn to_section_state<S: Subprotocol>(&self) -> SectionState {
        let h = self.get_handler(S::ID).expect("asm: unloaded subprotocol");
        h.to_section()
    }

    /// Exports each handler as a `SectionState` for constructing the final
    /// `AnchorState`, and returns both the sections and the accumulated logs.
    /// Consumes the manager.
    ///
    /// # Panics
    ///
    /// Panics if the exported sections are not sorted by `id`.
    pub(crate) fn export_sections_and_logs(self) -> (Vec<SectionState>, Vec<AsmLog>) {
        let sections = self
            .handlers
            .into_values()
            .map(|h| h.to_section())
            .collect::<Vec<_>>();

        // sanity check
        assert!(
            sections.is_sorted_by_key(|s| s.id),
            "asm: sections not sorted on export"
        );

        (sections, self.logs)
    }

    pub(crate) fn export_aux_requests(self) -> Vec<AuxRequest> {
        self.aux_requests
    }
}

impl SubprotoManager {
    pub(crate) fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
            logs: Vec::new(),
            aux_requests: Vec::new(),
        }
    }
}

impl MsgRelayer for SubprotoManager {
    fn relay_msg(&mut self, m: &dyn InterprotoMsg) {
        let h = self
            .get_handler_mut(m.id())
            .expect("asm: msg to unloaded subprotocol");
        h.accept_msg(m);
    }

    fn emit_log(&mut self, log: AsmLog) {
        self.logs.push(log);
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

impl AuxInputCollector for SubprotoManager {
    fn request_aux_input(&mut self, req: AuxRequest) {
        self.aux_requests.push(req);
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}
