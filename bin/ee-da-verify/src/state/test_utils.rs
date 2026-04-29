use alpen_ee_common::{BatchId, DaBlob, EvmHeaderSummary};

pub(crate) const TEST_CHAIN_SPEC: &str = "dev";
pub(crate) const MAX_SEQUENCE_INCREMENT: u64 = 10;
pub(crate) const MAX_SEQUENCE_LEN: usize = 8;
pub(crate) const MAX_SEQUENCE_INCREMENT_SUM: u64 = MAX_SEQUENCE_INCREMENT * MAX_SEQUENCE_LEN as u64;
pub(crate) const MAX_LINKAGE_STEP: u64 = 5;

pub(crate) fn test_blob(prev_block: [u8; 32], last_block: [u8; 32], block_num: u64) -> DaBlob {
    let gas_used = block_num % 1_000;
    DaBlob {
        batch_id: BatchId::from_parts(prev_block.into(), last_block.into()),
        evm_header: EvmHeaderSummary {
            block_num,
            timestamp: block_num,
            base_fee: block_num,
            gas_used,
            gas_limit: gas_used.saturating_add(1),
        },
        state_diff: Default::default(),
    }
}

pub(crate) fn hash_from_seed(seed: u64) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash[24..].copy_from_slice(&seed.to_be_bytes());
    hash
}

pub(crate) fn make_block_numbers(start: u64, increments: &[u64]) -> Vec<u64> {
    let mut block_numbers = Vec::with_capacity(increments.len());
    let mut current = start;
    for increment in increments {
        current = current.saturating_add(*increment);
        block_numbers.push(current);
    }
    block_numbers
}

pub(crate) fn make_linked_blobs(genesis_anchor: [u8; 32], block_numbers: &[u64]) -> Vec<DaBlob> {
    let mut blobs = Vec::with_capacity(block_numbers.len());
    let mut prev_block = genesis_anchor;

    for (index, block_num) in block_numbers.iter().enumerate() {
        let last_block = hash_from_seed(index as u64);
        blobs.push(test_blob(prev_block, last_block, *block_num));
        prev_block = last_block;
    }

    blobs
}
