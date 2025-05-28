//! Subprotocol handler.

use std::{any::Any, collections::BTreeMap};

use strata_asm_common::{
    AsmError, InterprotoMsg, Log, MsgRelayer, SectionState, SubprotoHandler, Subprotocol,
    SubprotocolId, TxInput,
};

/// Wrapper around the common subprotocol interface that handles the common
/// buffering logic for interproto messages.
pub(crate) struct HandlerImpl<S: Subprotocol, R> {
    state: S::State,
    interproto_msg_buf: Vec<S::Msg>,

    _r: std::marker::PhantomData<R>,
}

impl<S: Subprotocol + 'static, R: MsgRelayer + 'static> HandlerImpl<S, R> {
    pub(crate) fn new(state: S::State, interproto_msg_buf: Vec<S::Msg>) -> Self {
        Self {
            state,
            interproto_msg_buf,
            _r: std::marker::PhantomData,
        }
    }

    /// Constructs an instance by wrapping a subprotocol's state.
    pub(crate) fn from_state(state: S::State) -> Self {
        Self::new(state, Vec::new())
    }
}

impl<S: Subprotocol, R: MsgRelayer> SubprotoHandler for HandlerImpl<S, R> {
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

    fn process_txs(&mut self, txs: &[TxInput<'_>], relayer: &mut dyn MsgRelayer) {
        let relayer = relayer
            .as_mut_any()
            .downcast_mut::<R>()
            .expect("asm: handler");
        S::process_txs(&mut self.state, txs, relayer);
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
    logs: Vec<Log>,
}

impl SubprotoManager {
    /// Inserts a subproto by creating a handler for it, wrapping a tstate.
    pub(crate) fn insert_subproto<S: Subprotocol>(&mut self, state: S::State) {
        let handler = HandlerImpl::<S, Self>::from_state(state);
        assert_eq!(
            handler.id(),
            S::ID,
            "asm: subproto handler impl ID doesn't match"
        );
        self.insert_handler(Box::new(handler));
    }

    /// Dispatches transaction processing to the appropriate handler.
    ///
    /// This default implementation temporarily removes the handler to satisfy
    /// borrow-checker constraints, invokes `process_txs` with `self` as the relayer,
    /// and then reinserts the handler.
    pub(crate) fn invoke_process_txs<S: Subprotocol>(&mut self, txs: &[TxInput<'_>]) {
        // We temporarily take the handler out of the map so we can call
        // `process_txs` with `self` as the relayer without violating the
        // borrow checker.
        let mut h = self
            .remove_handler(S::ID)
            .expect("asm: unloaded subprotocol");
        h.process_txs(txs, self);
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

    /// Exports each handler as a section we can use when constructing the final
    /// `AnchorState`.  Consumes the manager.
    pub(crate) fn export_sections(self) -> Vec<SectionState> {
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

        sections
    }
}

impl SubprotoManager {
    pub(crate) fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
            logs: Vec::new(),
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

    fn emit_log(&mut self, log: Log) {
        self.logs.push(log);
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}
