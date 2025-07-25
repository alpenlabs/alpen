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
    genesis_registry: &'a GenesisConfigRegistry,
}

impl<'a, 'x> SubprotoLoaderStage<'a, 'x> {
    pub(crate) fn new(
        anchor_state: &'a AnchorState,
        manager: &'a mut SubprotoManager,
        aux_bundle: &'x BTreeMap<SubprotocolId, Vec<AuxPayload>>,
        genesis_registry: &'a GenesisConfigRegistry,
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
                // Deserialize genesis config from registry, or use default if not found
                let genesis_config: S::GenesisConfig =
                    self.genesis_registry.get(S::ID).unwrap_or_else(|| {
                        // This is expected behavior: forces upper layer ASM managers to provide
                        // genesis config for subprotocols that require specific genesis types.
                        // For subprotocols that use () or empty structs as GenesisConfig, this
                        // fallback will work fine. For subprotocols with non-empty genesis config
                        // requirements, this will panic, ensuring proper configuration is provided.
                        borsh::from_slice(&[])
                            .expect("asm: subprotocol requires genesis config but none provided")
                    });

                S::init(genesis_config).expect("asm: failed to initialize subprotocol state")
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
