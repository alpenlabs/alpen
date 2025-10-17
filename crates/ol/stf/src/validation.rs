use strata_crypto::verify_schnorr_sig;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLLog, compute_logs_root};
use strata_ol_state_types::OLState;
use strata_primitives::{CredRule, params::RollupParams};

use crate::error::BlockValidationError;

/// Block validation before execution. Checks continuity, signature etc.
pub fn pre_exec_block_validate(
    block: &OLBlock,
    prev_header: &OLBlockHeader,
    params: &RollupParams,
) -> Result<(), BlockValidationError> {
    // validate signature
    validate_block_signature(params, block)?;

    let cur_header = block.signed_header().header();

    // Check slot continuity
    // TODO: What to do with genesis block? I don't want to have Option<> lingering around.
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

    // Check epoch progression - epoch should not decrease and increase only by 1 at max
    let epoch_delta = (cur_header.epoch() as i64) - (prev_header.epoch() as i64);
    let valid_increment = epoch_delta == 0 || epoch_delta == 1;
    if !valid_increment {
        return Err(BlockValidationError::InvalidEpoch(cur_header.epoch()));
    }

    // Check timestamp progression - should not go backwards
    // TODO: use threshold like Bitcoin if needed
    if cur_header.timestamp() < prev_header.timestamp() {
        return Err(BlockValidationError::InvalidTimestamp(
            cur_header.timestamp(),
        ));
    }

    // Validate body root
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

/// Block validation after block execution like state root and logs root checks.
pub fn post_exec_validation(
    block: &OLBlock,
    new_state: &OLState,
    stf_logs: &[OLLog],
) -> Result<(), BlockValidationError> {
    // Check state root matches.
    let expected = block.header().state_root();
    let got = new_state.compute_root();
    if expected != got {
        return Err(BlockValidationError::StateRootMismatch { expected, got });
    }

    // Check logs root
    let expected = block.header().logs_root();
    let got = compute_logs_root(stf_logs);
    if expected != got {
        return Err(BlockValidationError::LogsRootMismatch { expected, got });
    }

    Ok(())
}
