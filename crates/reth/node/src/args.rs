use alpen_reth_evm::evm::AlpenEvmFactory;

#[derive(Debug, Clone, Default)]
pub struct AlpenNodeArgs {
    pub sequencer_http: Option<String>,
    pub evm_factory: AlpenEvmFactory,
}
