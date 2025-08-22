use std::collections::BTreeMap;

use strata_l1_txfmt::{MagicBytes, SubprotocolId};

use crate::{AnchorState, AuxPayload, SubprotoHandler, Subprotocol};

/// A type-safe genesis configuration provider for a specific subprotocol.
pub trait GenesisProvider<S: Subprotocol> {
    /// Provide the genesis configuration for this subprotocol
    fn genesis_config(&self) -> &S::GenesisConfig;
}

/// Specification for a concrete ASM instantiation describing the subprotocols we
/// want to invoke and in what order.
///
/// This way, we only have to declare the subprotocols a single time and they
/// will always be processed in a consistent order as defined by an `AsmSpec`.
pub trait AsmSpec {
    /// 4-byte magic identifier for the SPS-50 L1 transaction header.
    fn magic_bytes(&self) -> MagicBytes;

    fn load_subprotocol_handlers(
        &self,
        pre_state: &AnchorState,
        aux_bundle: &BTreeMap<SubprotocolId, Vec<AuxPayload>>,
    ) -> BTreeMap<SubprotocolId, Box<dyn SubprotoHandler>>;
}
