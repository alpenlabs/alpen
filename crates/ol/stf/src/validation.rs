use strata_crypto::verify_schnorr_sig;
use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLLog, compute_logs_root};
use strata_params::RollupParams;
use strata_primitives::{Buf32, CredRule};

use crate::error::BlockValidationError;

/// Block validation before execution. Checks continuity, signature etc.
pub fn pre_exec_block_validate(
    block: &OLBlock,
    prev_header: &OLBlockHeader,
    params: &RollupParams,
) -> Result<(), BlockValidationError> {
    let cur_header = block.signed_header().header();

    validate_block_signature(params, block)?;
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

#[cfg(test)]
mod tests {
    use strata_ol_chain_types_new::{OLBlockBody, OLTxSegment, SignedOLBlockHeader};
    use strata_primitives::{Buf64, Epoch};

    use super::*;

    fn create_test_header(
        slot: u64,
        epoch: Epoch,
        timestamp: u64,
        state_root: Buf32,
    ) -> OLBlockHeader {
        OLBlockHeader::new(
            timestamp,
            slot,
            epoch,
            Buf32::zero(), // parent_blkid
            Buf32::zero(), // body_root
            Buf32::zero(), // logs_root
            state_root,
        )
    }

    fn create_block_with_header(header: OLBlockHeader) -> OLBlock {
        let signed = SignedOLBlockHeader::new(header, Buf64::zero());
        let body = OLBlockBody::new(OLTxSegment::new(vec![]), None);
        OLBlock::new(signed, body)
    }

    fn test_rollup_params() -> RollupParams {
        use bitcoin::absolute;
        use strata_btc_types::GenesisL1View;
        use strata_params::{OperatorConfig, ProofPublishMode};
        use strata_predicate::{PredicateKey, PredicateTypeId};
        use strata_primitives::L1BlockCommitment;

        RollupParams {
            magic_bytes: *b"TEST",
            block_time: 1000,
            da_tag: "da".to_string(),
            checkpoint_tag: "ckpt".to_string(),
            cred_rule: CredRule::Unchecked,
            genesis_l1_view: GenesisL1View {
                blk: L1BlockCommitment::new(
                    absolute::Height::ZERO,
                    strata_primitives::L1BlockId::from(Buf32::zero()),
                ),
                next_target: 0x1d00ffff,
                epoch_start_timestamp: 0,
                last_11_timestamps: [0; 11],
            },
            operator_config: OperatorConfig::Static(vec![]),
            evm_genesis_block_hash: Buf32::zero(),
            evm_genesis_block_state_root: Buf32::zero(),
            l1_reorg_safe_depth: 6,
            target_l2_batch_size: 100,
            max_address_length: 64,
            deposit_amount: bitcoin::Amount::from_sat(1000),
            checkpoint_predicate: PredicateKey::new(PredicateTypeId::AlwaysAccept, vec![]),
            dispatch_assignment_dur: 100,
            proof_publish_mode: ProofPublishMode::Timeout(300),
            max_deposits_in_block: 10,
            network: bitcoin::Network::Regtest,
        }
    }

    #[test]
    fn test_valid_block_passes_validation() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 0, 100, prev_root);

        let cur_root = Buf32::from([2u8; 32]);
        let cur_header = create_test_header(11, 0, 101, cur_root);
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_continuity_violation() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 0, 100, prev_root);

        // Jump from slot 10 to 15 (should be 11)
        let cur_header = create_test_header(15, 0, 101, prev_root);
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_err());
        match result.unwrap_err() {
            BlockValidationError::SlotMismatch { expected, got } => {
                assert_eq!(expected, 11);
                assert_eq!(got, 15);
            }
            err => panic!("Expected SlotMismatch, got {:?}", err),
        }
    }

    #[test]
    fn test_invalid_parent_blkid() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 0, 100, prev_root);

        // Create header with wrong parent_blkid
        let cur_header = OLBlockHeader::new(
            101,
            11,
            0,
            Buf32::from([99u8; 32]), // Wrong parent
            Buf32::zero(),
            Buf32::zero(),
            prev_root,
        );
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_err());
        match result.unwrap_err() {
            BlockValidationError::BlockIdMismatch { expected, got } => {
                assert_eq!(expected, prev_header.compute_root());
                assert_eq!(got, Buf32::from([99u8; 32]));
            }
            err => panic!("Expected BlockIdMismatch, got {:?}", err),
        }
    }

    #[test]
    fn test_epoch_cannot_jump() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 5, 100, prev_root);

        // Jump from epoch 5 to 10
        let cur_header = create_test_header(11, 10, 101, prev_root);
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_err());
        match result.unwrap_err() {
            BlockValidationError::InvalidEpoch(epoch) => {
                assert_eq!(epoch, 10);
            }
            err => panic!("Expected InvalidEpoch, got {:?}", err),
        }
    }

    #[test]
    fn test_epoch_can_stay_same() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 5, 100, prev_root);

        // Epoch stays at 5
        let cur_header = create_test_header(11, 5, 101, prev_root);
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_ok());
    }

    #[test]
    fn test_epoch_can_increment_by_one() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 5, 100, prev_root);

        // Epoch increments to 6
        let cur_header = create_test_header(11, 6, 101, prev_root);
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_ok());
    }

    #[test]
    fn test_timestamp_cannot_go_backward() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 0, 1000, prev_root);

        // Timestamp goes backward
        let cur_header = create_test_header(11, 0, 500, prev_root);
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_err());
        match result.unwrap_err() {
            BlockValidationError::InvalidTimestamp(ts) => {
                assert_eq!(ts, 500);
            }
            err => panic!("Expected InvalidTimestamp, got {:?}", err),
        }
    }

    #[test]
    fn test_timestamp_can_stay_same() {
        let prev_root = Buf32::from([1u8; 32]);
        let prev_header = create_test_header(10, 0, 1000, prev_root);

        // Timestamp stays the same
        let cur_header = create_test_header(11, 0, 1000, prev_root);
        let block = create_block_with_header(cur_header);

        let params = test_rollup_params();
        let result = pre_exec_block_validate(&block, &prev_header, &params);

        assert!(result.is_ok());
    }
}
