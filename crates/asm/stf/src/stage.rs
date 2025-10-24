//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_aux::{AuxRequestTable, verify_aux_input};
use strata_asm_common::{
    AnchorState, AuxDataTable, AuxInput, Stage, Subprotocol, SubprotocolId, TxInputRef,
};

use crate::manager::SubprotoManager;

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct PreProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    anchor_state: &'c AnchorState,
    tx_bufs: &'c BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,

    /// Aux requests table we write requests into.
    aux_requests: &'c mut AuxRequestTable,
}

impl<'c> PreProcessStage<'c> {
    pub(crate) fn new(
        manager: &'c mut SubprotoManager,
        anchor_state: &'c AnchorState,
        tx_bufs: &'c BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
        aux_requests: &'c mut AuxRequestTable,
    ) -> Self {
        Self {
            manager,
            anchor_state,
            tx_bufs,
            aux_requests,
        }
    }
}

impl Stage for PreProcessStage<'_> {
    fn invoke_subprotocol<S: Subprotocol>(&mut self) {
        let txs = self
            .tx_bufs
            .get(&S::ID)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let req = self
            .manager
            .invoke_pre_process_txs::<S>(txs, self.anchor_state);

        if !req.is_empty() {
            self.aux_requests.insert(S::ID, req);
        }
    }
}

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct ProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    anchor_state: &'c AnchorState,
    tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    aux_inputs: &'c AuxDataTable,
}

impl<'c> ProcessStage<'c> {
    pub(crate) fn new(
        manager: &'c mut SubprotoManager,
        anchor_state: &'c AnchorState,
        tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
        aux_inputs: &'c AuxDataTable,
    ) -> Self {
        Self {
            manager,
            anchor_state,
            tx_bufs,
            aux_inputs,
        }
    }
}

impl Stage for ProcessStage<'_> {
    fn invoke_subprotocol<S: Subprotocol>(&mut self) {
        let txs = self
            .tx_bufs
            .get(&S::ID)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let default_aux_data = AuxInput::default();
        let subprotocol_aux_data = self.aux_inputs.get(&S::ID).unwrap_or(&default_aux_data);

        if let Err(err) = verify_aux_input(
            subprotocol_aux_data,
            &self.anchor_state.chain_view.history_mmr,
            S::ID,
        ) {
            panic!("{err}");
        }

        self.manager
            .invoke_process_txs::<S>(txs, self.anchor_state, subprotocol_aux_data);
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
    fn invoke_subprotocol<S: Subprotocol>(&mut self) {
        self.manager.invoke_process_msgs::<S>();
    }
}
