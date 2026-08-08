//! GChain processor indirection wrappers.

use strata_gchain_types::*;

pub trait GChainProcDyn<S: GChainSpec>: 'static {
    // TODO add methods as needed
    fn on_init(&self, cur_node: &NodeRef<S>, node: &Node<S>) -> anyhow::Result<()>;
}

/// Generic processor shim wrapper to expose as `dyn`-safe object.
struct ProcShim<P: GChainProc> {
    proc: P,
}

impl<S: GChainSpec, P: GChainProc<Spec = S>> GChainProcDyn<S> for ProcShim<P> {
    fn on_init(&self, cur_node: &NodeRef<S>, node: &Node<S>) -> anyhow::Result<()> {
        self.proc.on_init(cur_node, node)
    }
}
