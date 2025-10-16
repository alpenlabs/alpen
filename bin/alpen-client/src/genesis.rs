use alloy_primitives::B256;
use reth_chainspec::ChainSpec;

pub(crate) struct BlockInfo {
    pub blockhash: B256,
    pub stateroot: B256,
}

pub(crate) fn ee_genesis_block_info(chain_spec: &ChainSpec) -> BlockInfo {
    let genesis_header = chain_spec.genesis_header();
    let genesis_stateroot = genesis_header.state_root;
    let genesis_hash = chain_spec.genesis_hash();

    BlockInfo {
        blockhash: genesis_hash,
        stateroot: genesis_stateroot,
    }
}
