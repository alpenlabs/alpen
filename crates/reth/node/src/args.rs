use std::sync::{atomic::AtomicU64, Arc};

use alpen_reth_evm::evm::AlpenEvmFactory;

#[derive(Debug, Clone)]
pub struct AlpenNodeArgs {
    pub sequencer_http: Option<String>,
    pub evm_factory: AlpenEvmFactory,
    /// Live DA rate (wei per byte) shared into the payload builder; sampled and frozen
    /// per block. Updated out of band from the sequencer's Bitcoin fee rate.
    pub live_da_rate: Arc<AtomicU64>,
}
