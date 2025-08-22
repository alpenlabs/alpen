//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AsmSpec, AuxPayload, GenesisProvider, Stage, Subprotocol, SubprotocolId,
    TxInputRef,
};

use crate::manager::SubprotoManager;

/// Stage to process txs pre-extracted from the block for each subprotocol.
pub(crate) struct PreProcessStage<'a, 'b, 'm> {
    anchor_state: &'a AnchorState,
    tx_bufs: &'b BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
    manager: &'m mut SubprotoManager,
}

impl<'a, 'b, 'm> PreProcessStage<'a, 'b, 'm> {
    pub(crate) fn new(
        tx_bufs: &'b BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
        manager: &'m mut SubprotoManager,
        anchor_state: &'a AnchorState,
    ) -> Self {
        Self {
            anchor_state,
            tx_bufs,
            manager,
        }
    }
}

impl<Spec: AsmSpec> Stage<Spec> for PreProcessStage<'_, '_, '_> {
    fn process_subprotocol<S: Subprotocol>(&mut self, _spec: &Spec)
    where
        Spec: GenesisProvider<S>,
    {
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
pub(crate) struct ProcessStage<'a, 'b, 'm> {
    anchor_state: &'a AnchorState,
    tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
    manager: &'m mut SubprotoManager,
}

impl<'a, 'b, 'm> ProcessStage<'a, 'b, 'm> {
    pub(crate) fn new(
        tx_bufs: BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
        manager: &'m mut SubprotoManager,
        anchor_state: &'a AnchorState,
    ) -> Self {
        Self {
            anchor_state,
            tx_bufs,
            manager,
        }
    }
}

impl<Spec: AsmSpec> Stage<Spec> for ProcessStage<'_, '_, '_> {
    fn process_subprotocol<S: Subprotocol>(&mut self, _spec: &Spec)
    where
        Spec: GenesisProvider<S>,
    {
        let txs = self
            .tx_bufs
            .get(&S::ID)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        self.manager.invoke_process_txs::<S>(txs, self.anchor_state);
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

impl<Spec: AsmSpec> Stage<Spec> for FinishStage<'_> {
    fn process_subprotocol<S: Subprotocol>(&mut self, _spec: &Spec)
    where
        Spec: GenesisProvider<S>,
    {
        self.manager.invoke_process_msgs::<S>();
    }
}
