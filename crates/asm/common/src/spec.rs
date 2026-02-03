use strata_l1_txfmt::MagicBytes;

use crate::{StateDeserializer, Subprotocol};

/// Specification for a concrete ASM instantiation describing the subprotocols we
/// want to invoke and in what order.
///
/// This way, we only have to declare the subprotocols a single time and they
/// will always be processed in a consistent order as defined by an `AsmSpec`.
pub trait AsmSpec {
    /// 4-byte magic identifier for the SPS-50 L1 transaction header.
    fn magic_bytes(&self) -> MagicBytes;

    /// Trigger to load the subprotocols into a manager.
    fn load_subprotocols(&self, loader: &mut impl Loader);

    /// Function that calls the stage with each subprotocol we intend to
    /// process, in the order we intend to process them.
    ///
    /// This MUST NOT change its behavior depending on the stage we're
    /// processing.
    fn call_subprotocols(&self, stage: &mut impl Stage);
}

pub trait Loader {
    /// Invoked by the ASM spec to perform logic to load the subprotocol for
    /// execution in this ASM invocation.
    fn load_subprotocol<S: Subprotocol>(&mut self, params: S::Params)
    where
        rkyv::Archived<S::State>: rkyv::Deserialize<S::State, StateDeserializer>;
}

/// Impl of a subprotocol execution stage.
pub trait Stage {
    /// Invoked by the ASM spec to perform a the stage's logic with respect to
    /// the subprotocol.
    fn invoke_subprotocol<S: Subprotocol>(&mut self);
}
