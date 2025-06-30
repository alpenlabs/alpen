use strata_l1_txfmt::MagicBytes;

use crate::Subprotocol;

/// Specification for a concrete ASM instantiation describing the subprotocols we
/// want to invoke and in what order.
///
/// This way, we only have to declare the subprotocols a single time and they
/// will always be processed in a consistent order as defined by an `AsmSpec`.
pub trait AsmSpec {
    /// Function that calls the loader with each subprotocol we intend to
    /// process, in the order we intend to process them.
    fn call_subprotocols(stage: &mut impl Stage);

    /// Returns the 4-byte magic identifier for the SPS-50 L1 transaction header.
    fn magic_bytes() -> MagicBytes;
}

/// Implementation of a subprotocol handling stage.
pub trait Stage {
    /// Invoked by the ASM spec to perform logic relating to a specific subprotocol.
    fn process_subprotocol<S: Subprotocol>(&mut self);
}
