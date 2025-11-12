//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AuxData, AuxDataProvider, AuxRequestCollector, AuxRequests, Stage, Subprotocol,
    SubprotocolId, TxInputRef,
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
        let aux_collector = AuxRequestCollector::new();
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
            .invoke_pre_process_txs::<S>(txs, self.anchor_state);
    }
}

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct ProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    anchor_state: &'c AnchorState,
    tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    aux_provider: AuxDataProvider,
}

impl<'c> ProcessStage<'c> {
    pub(crate) fn new(
        manager: &'c mut SubprotoManager,
        anchor_state: &'c AnchorState,
        tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
        aux_data: &'c AuxData,
    ) -> Self {
        // Create a single aux provider for all subprotocols
        let aux_provider =
            AuxDataProvider::try_new(aux_data, &anchor_state.chain_view.manifest_mmr)
                .expect("asm: failed to create aux provider");

        Self {
            manager,
            anchor_state,
            tx_bufs,
            aux_provider,
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
            .invoke_process_txs::<S>(txs, self.anchor_state, &self.aux_provider);
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
