//! Subprotocol handler.

use std::{any::Any, collections::BTreeMap};

use borsh::BorshDeserialize;
use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, AuxInputCollector, AuxRequest, InterprotoMsg, Loader,
    MsgRelayer, SectionState, SubprotoHandler, Subprotocol, SubprotocolId, TxInputRef,
};

/// Wrapper around the common subprotocol interface that handles the common
/// buffering logic for interproto messages.
pub(crate) struct HandlerImpl<S: Subprotocol, R, C> {
    params: S::Params,
    state: S::State,
    interproto_msg_buf: Vec<S::Msg>,

    _r: std::marker::PhantomData<R>,
    _c: std::marker::PhantomData<C>,
}

impl<S: Subprotocol + 'static, R: MsgRelayer + 'static, C: AuxInputCollector + 'static>
    HandlerImpl<S, R, C>
{
    pub(crate) fn new(params: S::Params, state: S::State, interproto_msg_buf: Vec<S::Msg>) -> Self {
        Self {
            params,
            state,
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

    // TODO make this just return the aux request
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
        S::pre_process_txs(&self.state, txs, collector, anchor_pre, &self.params);
    }

    fn process_txs(
        &mut self,
        txs: &[TxInputRef<'_>],
        relayer: &mut dyn MsgRelayer,
        anchor_pre: &AnchorState,
        aux_input_data: &[u8],
    ) {
        let relayer = relayer
            .as_mut_any()
            .downcast_mut::<R>()
            .expect("asm: handler");

        // TODO better error handling
        let parsed_aux = <S::AuxInput as BorshDeserialize>::try_from_slice(aux_input_data)
            .map_err(|e| AsmError::Deserialization(S::ID, e))
            .unwrap();

        S::process_txs(
            &mut self.state,
            txs,
            anchor_pre,
            &parsed_aux,
            relayer,
            &self.params,
        );
    }

    fn process_buffered_msgs(&mut self) {
        // TODO probably will make this more sophisticated
        S::process_msgs(&mut self.state, &self.interproto_msg_buf, &self.params)
    }

    fn to_section(&self) -> SectionState {
        SectionState::from_state::<S>(&self.state)
    }
}

/// Manages subproto handlers and relays messages between them.
pub(crate) struct SubprotoManager {
    handlers: BTreeMap<SubprotocolId, Box<dyn SubprotoHandler>>,
    logs: Vec<AsmLogEntry>,
}

impl SubprotoManager {
    /// Inserts a subproto by creating a handler for it, wrapping a tstate.
    pub(crate) fn insert_subproto<S: Subprotocol>(&mut self, params: S::Params, state: S::State) {
        let handler = HandlerImpl::<S, Self, BasicAuxCollector>::new(params, state, Vec::new());
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
    ) -> Option<AuxRequest> {
        // We temporarily take the handler out of the map so we can call
        // `process_txs` with `self` as the relayer without violating the
        // borrow checker.
        let mut h = self
            .remove_handler(S::ID)
            .expect("asm: unloaded subprotocol");

        // Invoke the preprocess function.
        let mut acol = BasicAuxCollector::new();
        h.pre_process_txs(txs, &mut acol, anchor_pre);
        self.insert_handler(h);

        acol.into_request()
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
        aux_input_data: &[u8],
    ) {
        // We temporarily take the handler out of the map so we can call
        // `process_txs` with `self` as the relayer without violating the
        // borrow checker.
        let mut h = self
            .remove_handler(S::ID)
            .expect("asm: unloaded subprotocol");
        h.process_txs(txs, self, anchor_pre, aux_input_data);
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
    pub(crate) fn export_sections_and_logs(self) -> (Vec<SectionState>, Vec<AsmLogEntry>) {
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

    fn emit_log(&mut self, log: AsmLogEntry) {
        self.logs.push(log);
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Shim for just remembering the aux request given to us.
pub(crate) struct BasicAuxCollector {
    req: Option<AuxRequest>,
}

impl BasicAuxCollector {
    pub(crate) fn new() -> Self {
        Self { req: None }
    }

    pub(crate) fn into_request(self) -> Option<AuxRequest> {
        self.req
    }
}

impl AuxInputCollector for BasicAuxCollector {
    fn request_aux_input(&mut self, req: AuxRequest) {
        self.req = Some(req);
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Basic subprotocol loader impl to be passed to spec impls.
pub(crate) struct AnchorStateLoader<'c> {
    anchor: &'c AnchorState,
    man: &'c mut SubprotoManager,
}

impl<'c> AnchorStateLoader<'c> {
    pub(crate) fn new(anchor: &'c AnchorState, man: &'c mut SubprotoManager) -> Self {
        Self { anchor, man }
    }
}

impl<'c> Loader for AnchorStateLoader<'c> {
    fn load_subprotocol<S: Subprotocol>(&mut self, params: S::Params) {
        // Load or create the subprotocol state.
        // OPTIMIZE: Linear scan is done every time to find the section
        let state = match self.anchor.find_section(S::ID) {
            Some(sec) => sec
                .try_to_state::<S>()
                .expect("asm: invalid section subproto state"),
            // State not found in the anchor state, which occurs in two scenarios:
            // 1. During genesis block processing, before any state initialization
            // 2. When introducing a new subprotocol to an existing chain
            // In either case, we must initialize a fresh state from the provided configuration
            // in the AsmSpec
            None => {
                // Just instantiate the subprotocol state from the params.
                S::init(&params).expect("asm: failed to construct new subproto state")
            }
        };

        self.man.insert_subproto::<S>(params, state);
    }
}
