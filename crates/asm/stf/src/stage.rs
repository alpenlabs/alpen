//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AuxPayload, GenesisConfigRegistry, Stage, Subprotocol, SubprotocolId, TxInputRef,
};

use crate::manager::SubprotoManager;

/// Stage that loads each subprotocol from the anchor state we're basing off of.
pub(crate) struct SubprotoLoaderStage<'a, 'x> {
    anchor_state: &'a AnchorState,
    manager: &'a mut SubprotoManager,
    aux_bundle: &'x BTreeMap<SubprotocolId, Vec<AuxPayload>>,
    genesis_registry: Option<&'a GenesisConfigRegistry>,
}

impl<'a, 'x> SubprotoLoaderStage<'a, 'x> {
    pub(crate) fn new(
        anchor_state: &'a AnchorState,
        manager: &'a mut SubprotoManager,
        aux_bundle: &'x BTreeMap<SubprotocolId, Vec<AuxPayload>>,
        genesis_registry: Option<&'a GenesisConfigRegistry>,
    ) -> Self {
        Self {
            anchor_state,
            manager,
            aux_bundle,
            genesis_registry,
        }
    }
}

impl Stage for SubprotoLoaderStage<'_, '_> {
    fn process_subprotocol<S: Subprotocol>(&mut self) {
        let state = match self.anchor_state.find_section(S::ID) {
            Some(sec) => sec
                .try_to_state::<S>()
                .expect("asm: invalid section subproto state"),
            // State not found in the anchor state, which occurs in two scenarios:
            // 1. During genesis block processing, before any state initialization
            // 2. When introducing a new subprotocol to an existing chain
            // In either case, we must initialize a fresh state from the provided configuration in
            // genesis_registry
            None => {
                // Try to get genesis config data from registry
                let genesis_config_data = if let Some(registry) = self.genesis_registry {
                    registry.get_raw(S::ID)
                } else {
                    None
                };

                S::init(genesis_config_data).expect("asm: failed to initialize subprotocol state")
            }
        };

        // Extract auxiliary inputs for this subprotocol from the bundle
        let aux_inputs = match self.aux_bundle.get(&S::ID) {
            Some(payloads) => payloads
                .iter()
                .map(|payload| {
                    payload
                        .try_to_aux_input::<S>()
                        .expect("asm: invalid aux input")
                })
                .collect(),
            None => Vec::new(),
        };

        self.manager.insert_subproto::<S>(state, aux_inputs);
    }
}

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

impl Stage for PreProcessStage<'_, '_, '_> {
    fn process_subprotocol<S: Subprotocol>(&mut self) {
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

impl Stage for ProcessStage<'_, '_, '_> {
    fn process_subprotocol<S: Subprotocol>(&mut self) {
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

impl Stage for FinishStage<'_> {
    fn process_subprotocol<S: Subprotocol>(&mut self) {
        self.manager.invoke_process_msgs::<S>();
    }
}
