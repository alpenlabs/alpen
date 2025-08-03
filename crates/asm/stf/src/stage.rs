//! Loader infrastructure for setting up the context.
// TODO maybe move (parts of) this module to common?

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AsmSpec, AuxPayload, GenesisProvider, Stage, Subprotocol, SubprotocolId, TxInputRef,
};

use crate::manager::SubprotoManager;

/// Stage that loads each subprotocol from the anchor state we're basing off of.
pub(crate) struct SubprotoLoaderStage<'a, 'x, S: AsmSpec> {
    anchor_state: &'a AnchorState,
    manager: &'a mut SubprotoManager,
    aux_bundle: &'x BTreeMap<SubprotocolId, Vec<AuxPayload>>,
    _phantom: std::marker::PhantomData<S>,
}

impl<'a, 'x, S: AsmSpec> SubprotoLoaderStage<'a, 'x, S> {
    pub(crate) fn new(
        anchor_state: &'a AnchorState,
        manager: &'a mut SubprotoManager,
        aux_bundle: &'x BTreeMap<SubprotocolId, Vec<AuxPayload>>,
    ) -> Self {
        Self {
            anchor_state,
            manager,
            aux_bundle,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<Spec: AsmSpec> Stage<Spec> for SubprotoLoaderStage<'_, '_, Spec> {
    fn process_subprotocol<S: Subprotocol>(&mut self)
    where
        Spec: GenesisProvider<S>,
    {
        // Load or create the subprotocol state.
        // OPTIMIZE: Linear scan is done every time to find the section
        let state = match self.anchor_state.find_section(S::ID) {
            Some(sec) => sec
                .try_to_state::<S>()
                .expect("asm: invalid section subproto state"),
            // State not found in the anchor state, which occurs in two scenarios:
            // 1. During genesis block processing, before any state initialization
            // 2. When introducing a new subprotocol to an existing chain
            // In either case, we must initialize a fresh state from the provided configuration
            // in the AsmSpec
            None => {
                // Get the type-safe genesis config from the AsmSpec
                let genesis_config = Spec::genesis_config_for::<S>();
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

impl<Spec: AsmSpec> Stage<Spec> for PreProcessStage<'_, '_, '_> {
    fn process_subprotocol<S: Subprotocol>(&mut self)
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
    fn process_subprotocol<S: Subprotocol>(&mut self)
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
    fn process_subprotocol<S: Subprotocol>(&mut self)
    where
        Spec: GenesisProvider<S>,
    {
        self.manager.invoke_process_msgs::<S>();
    }
}
