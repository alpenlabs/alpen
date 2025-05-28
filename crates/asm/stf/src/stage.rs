//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{AnchorState, Stage, Subprotocol, SubprotocolId, TxInput};

use crate::manager::SubprotoManager;

/// Stage that loads each subprotocol from the anchor state we're basing off of.
pub(crate) struct SubprotoLoaderStage<'a> {
    anchor_state: &'a AnchorState,
    manager: &'a mut SubprotoManager,
}

impl<'a> SubprotoLoaderStage<'a> {
    pub(crate) fn new(anchor_state: &'a AnchorState, manager: &'a mut SubprotoManager) -> Self {
        Self {
            anchor_state,
            manager,
        }
    }
}

impl Stage for SubprotoLoaderStage<'_> {
    fn process_subprotocol<S: Subprotocol>(&mut self) {
        // Load or create the subprotocol state.
        // OPTIMIZE: Linear scan is done every time to find the section
        let state = match self.anchor_state.find_section(S::ID) {
            Some(sec) => sec
                .try_to_state::<S>()
                .expect("asm: invalid section subproto state"),
            None => S::init(),
        };

        self.manager.insert_subproto::<S>(state);
    }
}

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct ProcessStage<'b, 'm> {
    tx_bufs: BTreeMap<SubprotocolId, Vec<TxInput<'b>>>,
    manager: &'m mut SubprotoManager,
}

impl<'b, 'm> ProcessStage<'b, 'm> {
    pub(crate) fn new(
        tx_bufs: BTreeMap<SubprotocolId, Vec<TxInput<'b>>>,
        manager: &'m mut SubprotoManager,
    ) -> Self {
        Self { tx_bufs, manager }
    }
}

impl Stage for ProcessStage<'_, '_> {
    fn process_subprotocol<S: Subprotocol>(&mut self) {
        let txs = self
            .tx_bufs
            .get(&S::ID)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        self.manager.invoke_process_txs::<S>(txs);
    }
}

/// Stage to handle messages exchanged between subprotocols in execution.
pub(crate) struct FinishStage<'m> {
    manager: &'m mut SubprotoManager,
}

impl<'m> FinishStage<'m> {
    pub(crate) fn new(manager: &'m mut SubprotoManager) -> Self {
        Self { manager }
    }
}

impl Stage for FinishStage<'_> {
    fn process_subprotocol<S: Subprotocol>(&mut self) {
        self.manager.invoke_process_msgs::<S>();
    }
}
