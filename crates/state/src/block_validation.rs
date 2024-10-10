use strata_crypto::verify_schnorr_sig;
use strata_primitives::{block_credential::CredRule, hash, params::RollupParams};
use tracing::warn;

use crate::{
    block::L2Block,
    header::{L2Header, SignedL2BlockHeader},
};

pub fn validate_block_segments(block: &L2Block) -> bool {
    // Check if the l1_segment_hash matches between L2Block and L2BlockHeader
    let l1seg_buf = borsh::to_vec(block.l1_segment()).expect("blockasm: enc l1 segment");
    let l1_segment_hash = hash::raw(&l1seg_buf);
    if l1_segment_hash != *block.header().l1_payload_hash() {
        warn!("computed l1_segment_hash doesn't match between L2Block and L2BlockHeader");
        return false;
    }

    // Check if the exec_segment_hash matches between L2Block and L2BlockHeader
    let eseg_buf = borsh::to_vec(block.exec_segment()).expect("blockasm: enc exec segment");
    let exec_segment_hash = hash::raw(&eseg_buf);
    if exec_segment_hash != *block.header().exec_payload_hash() {
        warn!("computed exec_segment_hash doesn't match between L2Block and L2BlockHeader");
        return false;
    }

    true
}

pub fn check_block_credential(header: &SignedL2BlockHeader, rollup_params: &RollupParams) -> bool {
    let sigcom = header.header().get_sighash();
    match &rollup_params.cred_rule {
        CredRule::Unchecked => true,
        CredRule::SchnorrKey(pubkey) => verify_schnorr_sig(header.sig(), &sigcom, pubkey),
    }
}
