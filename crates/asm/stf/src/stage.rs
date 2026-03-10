//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AuxRequestCollector, AuxRequests, Stage, Subprotocol, SubprotocolId, TxInputRef,
    VerifiedAuxData,
};
use strata_identifiers::L1BlockCommitment;

use crate::manager::SubprotoManager;

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct PreProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    tx_bufs: &'c BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    aux_collector: AuxRequestCollector,
}

impl<'c> PreProcessStage<'c> {
    pub(crate) fn new(
        manager: &'c mut SubprotoManager,
        anchor_state: &'c AnchorState,
        tx_bufs: &'c BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    ) -> Self {
        let accumulator = &anchor_state.chain_view.history_accumulator;
        let min_manifest_height = accumulator.offset();
        let max_manifest_height = accumulator.last_inserted_height();
        let aux_collector = AuxRequestCollector::new(min_manifest_height, max_manifest_height);
        Self {
            manager,
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
            .invoke_pre_process_txs::<S>(&mut self.aux_collector, txs);
    }
}

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct ProcessStage<'c> {
    manager: &'c mut SubprotoManager,
    l1ref: &'c L1BlockCommitment,
    tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
    verified_aux_data: VerifiedAuxData,
}

impl<'c> ProcessStage<'c> {
    pub(crate) fn new(
        manager: &'c mut SubprotoManager,
        l1ref: &'c L1BlockCommitment,
        tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'c>>>,
        verified_aux_data: VerifiedAuxData,
    ) -> Self {
        Self {
            manager,
            l1ref,
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
            .invoke_process_txs::<S>(txs, self.l1ref, &self.verified_aux_data);
    }
}

/// Stage to handle messages exchanged between subprotocols in execution.
pub(crate) struct FinishStage<'m> {
    manager: &'m mut SubprotoManager,
    l1ref: &'m L1BlockCommitment,
}

impl<'m> FinishStage<'m> {
    pub(crate) fn new(manager: &'m mut SubprotoManager, l1ref: &'m L1BlockCommitment) -> Self {
        Self { manager, l1ref }
    }
}

impl Stage for FinishStage<'_> {
    fn invoke_subprotocol<S: Subprotocol>(&mut self) {
        self.manager.invoke_process_msgs::<S>(self.l1ref);
    }
}
