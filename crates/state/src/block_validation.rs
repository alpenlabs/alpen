use strata_crypto::verify_schnorr_sig;
use strata_primitives::{
    block_credential::CredRule,
    buf::{Buf32, Buf64},
    hash,
    params::RollupParams,
};
use thiserror::Error;
use tracing::warn;

use crate::{
    block::L2Block,
    header::{L2Header, SignedL2BlockHeader},
};

/// Errors relating to block structural checks.
#[derive(Debug, Error)]
pub enum BlockCheckError {
    /// Indicates that hash references within the block don't match properly.
    #[error("structural hashes mismatch")]
    StructureInvalid,

    /// Indicates that the block's credential is invalid, for some reason.
    #[error("cred invalid")]
    CredentialInvalid,
}

/// Validates the segments in the block body match the header.
pub fn validate_block_structure(block: &L2Block) -> Result<(), BlockCheckError> {
    // Check if the l1_segment_hash matches between L2Block and L2BlockHeader
    let l1seg_buf = borsh::to_vec(block.l1_segment()).expect("blockasm: enc l1 segment");
    let l1_segment_hash = hash::raw(&l1seg_buf);
    if l1_segment_hash != *block.header().l1_payload_hash() {
        warn!("computed l1_segment_hash doesn't match between L2Block and L2BlockHeader");
        return Err(BlockCheckError::StructureInvalid);
    }

    // Check if the exec_segment_hash matches between L2Block and L2BlockHeader
    let eseg_buf = borsh::to_vec(block.exec_segment()).expect("blockasm: enc exec segment");
    let exec_segment_hash = hash::raw(&eseg_buf);
    if exec_segment_hash != *block.header().exec_payload_hash() {
        warn!("computed exec_segment_hash doesn't match between L2Block and L2BlockHeader");
        return Err(BlockCheckError::StructureInvalid);
    }

    Ok(())
}

/// Checks a block's credential.
pub fn check_block_credential(
    header: &SignedL2BlockHeader,
    rollup_params: &RollupParams,
) -> Result<(), BlockCheckError> {
    let sigcom = header.header().get_sighash();
    if !verify_sequencer_signature(rollup_params, &sigcom, header.sig()) {
        return Err(BlockCheckError::CredentialInvalid);
    }

    Ok(())
}

/// Verifies a sequencer signature for some arbitrary message according to the
/// params's sequencer cred rule.
///
/// Returns if the signature is valid or not.
pub fn verify_sequencer_signature(rollup_params: &RollupParams, msg: &Buf32, sig: &Buf64) -> bool {
    match &rollup_params.cred_rule {
        CredRule::Unchecked => true,
        CredRule::SchnorrKey(pubkey) => verify_schnorr_sig(sig, msg, pubkey),
    }
}
