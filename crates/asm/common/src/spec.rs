use strata_l1_txfmt::MagicBytes;

use crate::Subprotocol;

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

    /// Get genesis config for a specific subprotocol type.
    /// This provides compile-time type safety by ensuring each subprotocol
    /// has its corresponding genesis config type.
    fn genesis_config_for<S: Subprotocol>(&self) -> &S::GenesisConfig
    where
        Self: GenesisProvider<S>;

    /// Function that calls the loader with each subprotocol we intend to
    /// process, in the order we intend to process them.
    fn call_subprotocols(&self, stage: &mut impl Stage<Self>)
    where
        Self: Sized;
}

/// Implementation of a subprotocol handling stage.
pub trait Stage<Spec: AsmSpec> {
    /// Invoked by the ASM spec to perform logic relating to a specific subprotocol.
    fn process_subprotocol<S: Subprotocol>(&mut self, spec: &Spec)
    where
        Spec: GenesisProvider<S>;
}
