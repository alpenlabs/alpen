use strata_crypto::verify_schnorr_sig;
use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLLog, compute_logs_root};
use strata_params::RollupParams;
use strata_primitives::{Buf32, CredRule};

use crate::error::BlockValidationError;

/// Block validation before execution. Checks continuity, signature etc.
pub fn pre_exec_block_validate(
    state_accessor: &impl StateAccessor,
    block: &OLBlock,
    prev_header: &OLBlockHeader,
    params: &RollupParams,
) -> Result<(), BlockValidationError> {
    let cur_header = block.signed_header().header();

    validate_block_signature(params, block)?;
    validate_state_root_continuity(state_accessor, prev_header)?;
    validate_slot_continuity(cur_header, prev_header)?;
    validate_epoch_progression(cur_header, prev_header)?;
    validate_timestamp_progression(cur_header, prev_header)?;
    validate_body_root(block)?;

    Ok(())
}

fn validate_block_signature(
    params: &RollupParams,
    block: &OLBlock,
) -> Result<(), BlockValidationError> {
    let pubkey = match params.cred_rule {
        CredRule::SchnorrKey(key) => key,

        // In this case we always just assume true.
        CredRule::Unchecked => return Ok(()),
    };

    let header_root = block.signed_header().header().compute_root();

    if !verify_schnorr_sig(&block.signed_header().signature(), &header_root, &pubkey) {
        return Err(BlockValidationError::InvalidSignature);
    }
    Ok(())
}

fn validate_state_root_continuity(
    state_accessor: &impl StateAccessor,
    prev_header: &OLBlockHeader,
) -> Result<(), BlockValidationError> {
    let expected_pre_root = state_accessor.compute_state_root();
    let got_pre_root = prev_header.state_root();
    if expected_pre_root != got_pre_root {
        return Err(BlockValidationError::PreStateRootMismatch {
            expected: expected_pre_root,
            got: got_pre_root,
        });
    }
    Ok(())
}

fn validate_slot_continuity(
    cur_header: &OLBlockHeader,
    prev_header: &OLBlockHeader,
) -> Result<(), BlockValidationError> {
    if cur_header.slot() > 0 {
        if cur_header.slot() != prev_header.slot() + 1 {
            return Err(BlockValidationError::SlotMismatch {
                expected: prev_header.slot() + 1,
                got: cur_header.slot(),
            });
        }

        let prev_id = prev_header.compute_root();
        if cur_header.parent_blkid() != prev_id {
            return Err(BlockValidationError::BlockIdMismatch {
                expected: prev_id,
                got: cur_header.parent_blkid(),
            });
        }
    }
    Ok(())
}

fn validate_epoch_progression(
    cur_header: &OLBlockHeader,
    prev_header: &OLBlockHeader,
) -> Result<(), BlockValidationError> {
    let epoch_delta = (cur_header.epoch() as i64) - (prev_header.epoch() as i64);
    let valid_increment = epoch_delta == 0 || epoch_delta == 1;
    if !valid_increment {
        return Err(BlockValidationError::InvalidEpoch(cur_header.epoch()));
    }
    Ok(())
}

fn validate_timestamp_progression(
    cur_header: &OLBlockHeader,
    prev_header: &OLBlockHeader,
) -> Result<(), BlockValidationError> {
    // Check timestamp progression - should not go backwards
    // TODO: use threshold like Bitcoin if needed
    if cur_header.timestamp() < prev_header.timestamp() {
        return Err(BlockValidationError::InvalidTimestamp(
            cur_header.timestamp(),
        ));
    }
    Ok(())
}

fn validate_body_root(block: &OLBlock) -> Result<(), BlockValidationError> {
    let exp_root = block.signed_header().header().body_root();
    let body_root = block.body().compute_root();
    if exp_root != body_root {
        return Err(BlockValidationError::MismatchedBodyRoot {
            expected: exp_root,
            got: body_root,
        });
    }
    Ok(())
}

/// Block validation after block execution like state root and logs root checks.
pub fn post_exec_block_validate<S: StateAccessor>(
    block: &OLBlock,
    new_state_root: Buf32,
    stf_logs: &[OLLog],
) -> Result<(), BlockValidationError> {
    // Check state root matches.
    let expected = block.header().state_root();
    let got = new_state_root;
    if expected != got {
        return Err(BlockValidationError::PostStateRootMismatch { expected, got });
    }

    // Check logs root
    let expected = block.header().logs_root();
    let got = compute_logs_root(stf_logs);
    if expected != got {
        return Err(BlockValidationError::LogsRootMismatch { expected, got });
    }

    Ok(())
}
