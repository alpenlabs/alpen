//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.
//! The ASM specification defines which subprotocols are enabled, their genesis configurations,
//! and protocol-level parameters like magic bytes.

use std::collections::BTreeMap;

use strata_asm_common::{
    AnchorState, AsmSpec, AuxPayload, GenesisProvider, SubprotoHandler, Subprotocol,
};
use strata_asm_proto_bridge_v1::{BridgeV1Config, BridgeV1Subproto};
use strata_asm_proto_core::{CoreGenesisConfig, OLCoreSubproto};
use strata_l1_txfmt::{MagicBytes, SubprotocolId};

use crate::handler::HandlerImpl;

mod handler;

/// Specification for the Strata ASM protocol
#[derive(Debug)]
pub struct StrataAsmSpec {
    magic_bytes: MagicBytes,
    core_genesis: CoreGenesisConfig,
    bridge_v1_genesis: BridgeV1Config,
}

impl GenesisProvider<OLCoreSubproto> for StrataAsmSpec {
    fn genesis_config(&self) -> &CoreGenesisConfig {
        &self.core_genesis
    }
}

impl GenesisProvider<BridgeV1Subproto> for StrataAsmSpec {
    fn genesis_config(&self) -> &BridgeV1Config {
        &self.bridge_v1_genesis
    }
}

impl StrataAsmSpec {
    /// Creates a new ASM specification.
    pub fn new(
        magic_bytes: strata_l1_txfmt::MagicBytes,
        core_genesis: CoreGenesisConfig,
        bridge_v1_genesis: BridgeV1Config,
    ) -> Self {
        Self {
            magic_bytes,
            core_genesis,
            bridge_v1_genesis,
        }
    }

    pub fn load_state_and_aux<S: Subprotocol>(
        &self,
        pre_state: &AnchorState,
        aux_bundle: &BTreeMap<SubprotocolId, Vec<AuxPayload>>,
    ) -> (S::State, Vec<S::AuxInput>)
    where
        Self: GenesisProvider<S>,
    {
        // Load Core subprotocol
        let state = match pre_state.find_section(S::ID) {
            Some(section) => section
                .try_to_state::<S>()
                .expect("asm: invalid core section state"),
            None => {
                let genesis_config = <Self as GenesisProvider<S>>::genesis_config(self);
                S::init(genesis_config).expect("asm: failed to initialize core subprotocol")
            }
        };

        // Extract auxiliary inputs for this subprotocol from the bundle
        let aux_inputs = match aux_bundle.get(&S::ID) {
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

        (state, aux_inputs)
    }
}

impl AsmSpec for StrataAsmSpec {
    fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }

    fn load_subprotocol_handlers(
        &self,
        pre_state: &AnchorState,
        aux_bundle: &BTreeMap<SubprotocolId, Vec<AuxPayload>>,
    ) -> BTreeMap<SubprotocolId, Box<dyn SubprotoHandler>> {
        let mut handlers: BTreeMap<SubprotocolId, Box<dyn SubprotoHandler>> = BTreeMap::new();

        // Load Core subprotocol
        let (core_state, aux_inputs) =
            self.load_state_and_aux::<OLCoreSubproto>(pre_state, aux_bundle);
        handlers.insert(
            OLCoreSubproto::ID,
            Box::new(HandlerImpl::<OLCoreSubproto>::new(core_state, aux_inputs)),
        );

        // Load BridgeV1 subprotocol
        let (bridge_state, aux_inputs) =
            self.load_state_and_aux::<BridgeV1Subproto>(pre_state, aux_bundle);
        handlers.insert(
            BridgeV1Subproto::ID,
            Box::new(HandlerImpl::<BridgeV1Subproto>::new(
                bridge_state,
                aux_inputs,
            )),
        );

        handlers
    }
}
