//! GChain processor indirection wrappers.

use strata_gchain_types::{GChainProc, GChainSpec};

pub trait GChainProcDyn<S: GChainSpec>: 'static {
    // TODO add methods as needed
}

/// Generic processor shim wrapper to expose as `dyn`-safe object.
struct ProcShim<P: GChainProc> {
    proc: P,
}

impl<S: GChainSpec, P: GChainProc<Spec = S>> GChainProcDyn<S> for ProcShim<P> {
    // TODO
}
