//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AuxData, AuxRequestCollector, AuxRequests, Stage, Subprotocol, SubprotocolId,
    TxInputRef, VerifiedAuxData,
};

use crate::manager::SubprotoManager;

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct PreProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    anchor_state: &'c AnchorState,
    tx_bufs: &'c BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    aux_collector: AuxRequestCollector,
}

impl<'c> PreProcessStage<'c> {
    pub(crate) fn new(
        manager: &'c mut SubprotoManager,
        anchor_state: &'c AnchorState,
        tx_bufs: &'c BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    ) -> Self {
        let max_manifest_height = anchor_state
            .chain_view
            .history_accumulator
            .last_inserted_height();
        let aux_collector = AuxRequestCollector::new(max_manifest_height);
        Self {
            manager,
            anchor_state,
            tx_bufs,
            aux_collector,
        }
    }

    pub(crate) fn into_aux_requests(self) -> AuxRequests {
        self.aux_collector.into_requests()
    }
}

impl Stage for PreProcessStage<'_> {
    fn invoke_subprotocol<S: Subprotocol>(&mut self) {
        let txs = self
            .tx_bufs
            .get(&S::ID)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        self.manager
            .invoke_pre_process_txs::<S>(&mut self.aux_collector, txs, self.anchor_state);
    }
}

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct ProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    anchor_state: &'c AnchorState,
    tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    verified_aux_data: VerifiedAuxData,
}

impl<'c> ProcessStage<'c> {
    pub(crate) fn new(
        manager: &'c mut SubprotoManager,
        anchor_state: &'c AnchorState,
        tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
        aux_data: &'c AuxData,
    ) -> Self {
        // Create a single verified aux data for all subprotocols
        let verified_aux_data =
            VerifiedAuxData::try_new(aux_data, &anchor_state.chain_view.history_accumulator)
                .expect("asm: failed to create verified aux data");

        Self {
            manager,
            anchor_state,
            tx_bufs,
            verified_aux_data,
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

        self.manager
            .invoke_process_txs::<S>(txs, self.anchor_state, &self.verified_aux_data);
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
