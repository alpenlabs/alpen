//! Subprotocol handler.

use std::{any::Any, collections::BTreeMap, ops::Sub};

use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, AsmSpec, AsmSpec2, AuxInputCollector, AuxRequest,
    InterprotoMsg, MsgRelayer, SectionState, SubprotoHandler, Subprotocol, SubprotocolId,
    TxInputRef,
};

/// Manages subproto handlers and relays messages between them.
pub(crate) struct SubprotoManager {
    handlers: BTreeMap<SubprotocolId, Box<dyn SubprotoHandler>>,
    logs: Vec<AsmLogEntry>,
    aux_requests: Vec<AuxRequest>,
}

impl SubprotoManager {
    pub(crate) fn subproto_ids(&self) -> Vec<SubprotocolId> {
        self.handlers.keys().copied().collect()
    }
    /// Dispatches pre-processing to the appropriate handler.
    ///
    /// This method temporarily removes the handler from the internal map to satisfy
    /// Rust’s borrow rules, invokes its `pre_process_txs` implementation with
    /// `self` acting as the `AuxInputCollector`, and then reinserts the handler.
    pub(crate) fn invoke_pre_process_txs<'t>(
        &mut self,
        txs: &BTreeMap<SubprotocolId, Vec<TxInputRef<'t>>>,
        anchor_pre: &AnchorState,
    ) {
        let ids = self.subproto_ids();
        for id in ids {
            // We temporarily take the handler out of the map so we can call
            // `process_txs` with `self` as the relayer without violating the
            // borrow checker.
            let mut h = self.remove_handler(id).expect("asm: unloaded subprotocol");
            let relevant_txs = txs.get(&id).map(|v| v.as_slice()).unwrap_or(&[]);
            h.pre_process_txs(relevant_txs, self, anchor_pre);
            self.insert_handler(h);
        }
    }

    /// Dispatches transaction processing to the appropriate handler.
    ///
    /// This default implementation temporarily removes the handler to satisfy
    /// borrow-checker constraints, invokes `process_txs` with `self` as the relayer,
    /// and then reinserts the handler.
    pub(crate) fn invoke_process_txs<'t, 's>(
        &mut self,
        txs: &BTreeMap<SubprotocolId, Vec<TxInputRef<'t>>>,
        anchor_pre: &AnchorState,
    ) {
        let ids = self.subproto_ids();
        for id in ids {
            // We temporarily take the handler out of the map so we can call
            // `process_txs` with `self` as the relayer without violating the
            // borrow checker.
            let mut h = self.remove_handler(id).expect("asm: unloaded subprotocol");
            let relevant_txs = txs.get(&id).map(|v| v.as_slice()).unwrap_or(&[]);
            h.process_txs(relevant_txs, self, anchor_pre);
            self.insert_handler(h);
        }
    }

    /// Dispatches buffered inter-protocol message processing to the handler.
    pub(crate) fn invoke_process_msgs(&mut self) {
        let ids = self.subproto_ids();
        for id in ids {
            let h = self.get_handler_mut(id).expect("asm: unloaded subprotocol");
            h.process_buffered_msgs()
        }
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

    pub(crate) fn export_aux_requests(self) -> Vec<AuxRequest> {
        self.aux_requests
    }
}

impl SubprotoManager {
    pub(crate) fn new(spec: &impl AsmSpec2, pre_state: &AnchorState) -> Self {
        let handlers = spec.load_subprotocol_handlers(pre_state);
        Self {
            handlers,
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

    fn emit_log(&mut self, log: AsmLogEntry) {
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
