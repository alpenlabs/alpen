use std::time::{SystemTime, UNIX_EPOCH};

use alpen_express_primitives::{
    block_credential,
    buf::{Buf32, Buf64},
    params::{Params, RollupParams, SyncParams},
};
use alpen_express_state::{
    block::{L2Block, L2BlockAccessory, L2BlockBody, L2BlockBundle},
    chain_state::ChainState,
    client_state::ClientState,
    exec_env::ExecEnvState,
    header::{L2BlockHeader, L2Header, SignedL2BlockHeader},
    l1::{L1HeaderRecord, L1ViewState},
};
use bitcoin::hashes::Hash;

use crate::{bitcoin::get_btc_chain, ArbitraryGenerator};

pub fn gen_block(parent: Option<&SignedL2BlockHeader>) -> L2BlockBundle {
    let arb = ArbitraryGenerator::new();
    let header: L2BlockHeader = arb.generate();
    let body: L2BlockBody = arb.generate();
    let accessory: L2BlockAccessory = arb.generate();

    let current_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let block_idx = parent.map(|h| h.blockidx() + 1).unwrap_or(0);
    let prev_block = parent.map(|h| h.get_blockid()).unwrap_or_default();
    let timestamp = parent
        .map(|h| h.timestamp() + 100)
        .unwrap_or(current_timestamp);

    let header = L2BlockHeader::new(
        block_idx,
        timestamp,
        prev_block,
        &body,
        *header.state_root(),
    );
    let empty_sig = Buf64::zero();
    let signed_header = SignedL2BlockHeader::new(header, empty_sig);
    let block = L2Block::new(signed_header, body);
    L2BlockBundle::new(block, accessory)
}

pub fn gen_l2_chain(parent: Option<SignedL2BlockHeader>, blocks_num: usize) -> Vec<L2BlockBundle> {
    let mut blocks = Vec::new();
    let mut parent = match parent {
        Some(p) => p,
        None => {
            let p = gen_block(None);
            blocks.push(p.clone());
            p.header().clone()
        }
    };

    for _ in 0..blocks_num {
        let block = gen_block(Some(&parent));
        blocks.push(block.clone());
        parent = block.header().clone()
    }

    blocks
}
pub fn gen_params() -> Params {
    Params {
        rollup: RollupParams {
            rollup_name: "express".to_string(),
            block_time: 1000,
            cred_rule: block_credential::CredRule::Unchecked,
            horizon_l1_height: 40318,
            genesis_l1_height: 40320,
            evm_genesis_block_hash: Buf32(
                "0x37ad61cff1367467a98cf7c54c4ac99e989f1fbb1bc1e646235e90c065c565ba"
                    .parse()
                    .unwrap(),
            ),
            evm_genesis_block_state_root: Buf32(
                "0x351714af72d74259f45cd7eab0b04527cd40e74836a45abcae50f92d919d988f"
                    .parse()
                    .unwrap(),
            ),
            l1_reorg_safe_depth: 5,
            target_l2_batch_size: 64,
        },
        run: SyncParams {
            l2_blocks_fetch_limit: 1000,
            l1_follow_distance: 3,
            client_checkpoint_interval: 10,
        },
    }
}

pub fn gen_client_state(params: Option<&Params>) -> ClientState {
    let params = match params {
        Some(p) => p,
        None => &gen_params(),
    };
    ClientState::from_genesis_params(
        params.rollup.horizon_l1_height,
        params.rollup.genesis_l1_height,
    )
}

pub fn get_genesis_chainstate() -> ChainState {
    let params = gen_params();
    // Build the genesis block and genesis consensus states.
    let gblock = L2BlockBundle::genesis(&params);
    let genesis_blkid = gblock.header().get_blockid();

    let geui = gblock.exec_segment().update().input();
    let gees =
        ExecEnvState::from_base_input(geui.clone(), params.rollup.evm_genesis_block_state_root);

    let l1_block = get_btc_chain().get_header(params.rollup.genesis_l1_height as u32);
    let safe_block = L1HeaderRecord::new(
        bitcoin::consensus::serialize(&l1_block),
        Buf32::from(l1_block.merkle_root.as_raw_hash().to_byte_array()),
    );
    let l1vs = L1ViewState::new_at_horizon(params.rollup.horizon_l1_height, safe_block);
    ChainState::from_genesis(genesis_blkid, l1vs, gees)
}
