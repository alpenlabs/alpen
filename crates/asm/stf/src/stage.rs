//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_aux::verify_aux_input;
use strata_asm_common::{
    AnchorState, AuxDataTable, AuxRequestTable, Stage, Subprotocol, SubprotocolId, TxInputRef,
};

use crate::manager::SubprotoManager;

#[cfg(feature = "preprocess")]
/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct PreProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    anchor_state: &'c AnchorState,
    tx_bufs: &'c BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,

    /// Aux requests table we write requests into.
    aux_requests: &'c mut AuxRequestTable,
}

#[cfg(feature = "preprocess")]
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

#[cfg(feature = "preprocess")]
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

        let raw_aux = self.aux_inputs.get(&S::ID);
        let verified_aux = raw_aux
            .map(|raw| {
                verify_aux_input(raw, &self.anchor_state.chain_view.history_mmr, S::ID)
                    .unwrap_or_else(|err| panic!("failed to verify aux input: {err:?}"))
            })
            .unwrap_or_default();

        self.manager
            .invoke_process_txs::<S>(txs, self.anchor_state, &verified_aux);
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
