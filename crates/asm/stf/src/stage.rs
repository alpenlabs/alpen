//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, GenesisConfigRegistry, Stage, Subprotocol, SubprotocolId, TxInput,
};

use crate::manager::SubprotoManager;

/// Stage that loads each subprotocol from the anchor state we're basing off of.
pub(crate) struct SubprotoLoaderStage<'a> {
    anchor_state: &'a AnchorState,
    manager: &'a mut SubprotoManager,
    genesis_registry: Option<&'a GenesisConfigRegistry>,
}

impl<'a> SubprotoLoaderStage<'a> {
    pub(crate) fn new(
        anchor_state: &'a AnchorState,
        manager: &'a mut SubprotoManager,
        genesis_registry: Option<&'a GenesisConfigRegistry>,
    ) -> Self {
        Self {
            anchor_state,
            manager,
            genesis_registry,
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
            // State not found in the anchor state, which occurs in two scenarios:
            // 1. During genesis block processing, before any state initialization
            // 2. When introducing a new subprotocol to an existing chain
            // In either case, we must initialize a fresh state from the provided configuration in
            // genesis_registry
            None => {
                // Try to get genesis config data from registry, otherwise fail
                let genesis_config_data = self
                    .genesis_registry
                    .ok_or("asm: genesis registry not available for state init")
                    .and_then(|registry| {
                        registry
                            .get_raw(S::ID)
                            .ok_or("asm: missing specific config for subprotocol")
                    })
                    .expect("asm: cannot get genesis config data for subprotocol");

                S::init(genesis_config_data).expect("asm: failed to initialize subprotocol state")
            }
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
