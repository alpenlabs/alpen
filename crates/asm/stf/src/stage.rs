//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AuxPayload, AuxRequest, Stage, Subprotocol, SubprotocolId, TxInputRef,
};

use crate::manager::SubprotoManager;

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct PreProcessStage<'a, 'b, 'm> {
    manager: &'m mut SubprotoManager,
    anchor_state: &'a AnchorState,
    tx_bufs: &'b BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,

    /// Aux requests table we write requests into.
    aux_requests: &'m mut BTreeMap<SubprotocolId, AuxRequest>,
}

impl<'a, 'b, 'm> PreProcessStage<'a, 'b, 'm> {
    pub(crate) fn new(
        manager: &'m mut SubprotoManager,
        anchor_state: &'a AnchorState,
        tx_bufs: &'b BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
        aux_requests: &'m mut BTreeMap<SubprotocolId, AuxRequest>,
    ) -> Self {
        Self {
            manager,
            anchor_state,
            tx_bufs,
            aux_requests,
        }
    }
}

impl Stage for PreProcessStage<'_, '_, '_> {
    fn invoke_subprotocol<S: Subprotocol>(&mut self) {
        let txs = self
            .tx_bufs
            .get(&S::ID)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let req = self
            .manager
            .invoke_pre_process_txs::<S>(txs, self.anchor_state);

        if let Some(req) = req {
            self.aux_requests.insert(S::ID, req);
        }
    }
}

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct ProcessStage<'a, 'b, 'm> {
    manager: &'m mut SubprotoManager,
    anchor_state: &'a AnchorState,
    tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
    aux_inputs: &'m BTreeMap<SubprotocolId, AuxPayload>,
}

impl<'a, 'b, 'm> ProcessStage<'a, 'b, 'm> {
    pub(crate) fn new(
        manager: &'m mut SubprotoManager,
        anchor_state: &'a AnchorState,
        tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
        aux_inputs: &'m BTreeMap<SubprotocolId, AuxPayload>,
    ) -> Self {
        Self {
            manager,
            anchor_state,
            tx_bufs,
            aux_inputs,
        }
    }
}

impl Stage for ProcessStage<'_, '_, '_> {
    fn invoke_subprotocol<S: Subprotocol>(&mut self) {
        let txs = self
            .tx_bufs
            .get(&S::ID)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        // Extract the auxiliary input for this subprotocol from the bundle
        let aux_input_data = self
            .aux_inputs
            .get(&S::ID)
            .map(|a| a.data())
            .unwrap_or_default();

        self.manager
            .invoke_process_txs::<S>(txs, self.anchor_state, aux_input_data);
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
