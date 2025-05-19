//! Core logic of the Bitcoin Blockspace proof that will be proven

use bitcoin::{
    consensus::{deserialize, serialize},
    Block,
};
use borsh::{BorshDeserialize, BorshSerialize};
use strata_l1tx::filter::{indexer::index_block, types::TxFilterConfig};
use strata_primitives::l1::{L1TxProof, ProtocolOperation};
use zkaleido::ZkVmEnv;

use crate::{block::check_block_integrity, tx_indexer::ProverTxVisitorImpl};

/// Defines the result of scanning an L1 block.
/// Includes protocol-relevant data posted on L1 block.
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct BlockScanResult {
    /// Raw header of the block that we processed
    pub raw_header: [u8; 80],
    /// Protocol Operations that we found after scanning the block
    pub protocol_ops: Vec<ProtocolOperation>,
}

#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct BlockscanProofOutput {
    pub blockscan_results: Vec<BlockScanResult>,
    pub tx_filters: TxFilterConfig,
}

/// Represents the input data required for generating an L1Scan proof.
#[derive(Debug)]
pub struct BlockScanProofInput {
    /// Full block that we use scan.
    ///
    /// Inclusion proof will be created if there are witness transactions
    pub btc_blocks: Vec<Block>,
    /// Tx filters we use to scan this block
    pub tx_filters: TxFilterConfig,
}

pub fn process_blockscan_proof(zkvm: &impl ZkVmEnv) {
    // 1a. Read the count and transaction filters used to scan the block
    let btc_blocks_count: usize = zkvm.read_serde();
    let tx_filters: TxFilterConfig = zkvm.read_borsh();

    let mut blockscan_results = Vec::with_capacity(btc_blocks_count);

    for _ in 0..btc_blocks_count {
        // 1b. Read the full serialized block and deserialize it
        let serialized_block = zkvm.read_buf();
        let block: Block = deserialize(&serialized_block).expect("invalid block serialization");

        // 1c. Read inclusion proof
        let coinbase_inclusion_proof: Option<L1TxProof> = zkvm.read_borsh();

        // 2. Check that the content of the block is valid
        assert!(
            check_block_integrity(&block, &coinbase_inclusion_proof),
            "invalid block"
        );

        // 3. Index the block for protocol ops
        let protocol_ops: Vec<ProtocolOperation> =
            index_block(&block, ProverTxVisitorImpl::new, &tx_filters)
                .into_iter()
                .flat_map(|entry| entry.into_item())
                .collect();

        // 5. Create the blockscan result and append to blockscan results
        let raw_header = serialize(&block.header)
            .try_into()
            .expect("bitcoin block header is 80 bytes");
        let result = BlockScanResult {
            raw_header,
            protocol_ops,
        };
        blockscan_results.push(result);
    }

    // 6. Create the final output to be committed and commit the output
    let output = BlockscanProofOutput {
        blockscan_results,
        tx_filters,
    };
    zkvm.commit_borsh(&output);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_test_utils::{bitcoin_mainnet_segment::BtcChainSegment, l2::gen_params};
    use zkaleido::ZkVmProgram;
    use zkaleido_native_adapter::{NativeHost, NativeMachine};

    use super::*;
    use crate::program::BtcBlockspaceProgram;

    fn get_native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                process_blockscan_proof(zkvm);
                Ok(())
            })),
        }
    }

    #[test]
    fn test_for_blocks_before_segwit() {
        let params = gen_params();
        let rollup_params = params.rollup();
        let tx_filters = TxFilterConfig::derive_from(rollup_params).unwrap();

        let btc_chain = BtcChainSegment::load();
        let btc_blocks = (40321..40325)
            .map(|h| btc_chain.get_block_at(h).unwrap())
            .collect();
        let input = BlockScanProofInput {
            btc_blocks,
            tx_filters,
        };
        BtcBlockspaceProgram::prove(&input, &get_native_host()).unwrap();
    }

    #[test]
    fn test_for_empty_blocks() {
        let params = gen_params();
        let rollup_params = params.rollup();
        let tx_filters = TxFilterConfig::derive_from(rollup_params).unwrap();

        let input = BlockScanProofInput {
            btc_blocks: vec![],
            tx_filters,
        };
        BtcBlockspaceProgram::prove(&input, &get_native_host()).unwrap();
    }

    #[test]
    fn test_process_for_single_block_after_segwit() {
        let params = gen_params();
        let rollup_params = params.rollup();
        let tx_filters = TxFilterConfig::derive_from(rollup_params).unwrap();

        let btc_blocks = vec![BtcChainSegment::load_full_block()];
        let input = BlockScanProofInput {
            btc_blocks,
            tx_filters,
        };
        BtcBlockspaceProgram::prove(&input, &get_native_host()).unwrap();
    }
}
