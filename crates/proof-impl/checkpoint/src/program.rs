use k256::schnorr::SigningKey;
use ssz::{Decode, Encode};
use strata_asm_proto_checkpoint_types::CheckpointClaim;
use strata_bridge_params::BridgeParams;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_types::OLState;
use strata_predicate::{PredicateKey, PredicateTypeId};
use zkaleido::{PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::NativeHost;

use crate::statements::process_ol_stf;

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x01u8; 32]).expect("valid test signing key")
}

#[derive(Debug)]
pub struct CheckpointProverInput {
    pub start_state: OLState,
    pub blocks: Vec<OLBlock>,
    pub parent: OLBlockHeader,
    pub da_state_diff_bytes: Vec<u8>,
    pub bridge_params: BridgeParams,
}

#[derive(Debug)]
pub struct CheckpointProgram;

impl ZkVmProgram for CheckpointProgram {
    type Input = CheckpointProverInput;
    type Output = CheckpointClaim;

    fn name() -> String {
        "Checkpoint".to_string()
    }

    fn proof_type() -> zkaleido::ProofType {
        zkaleido::ProofType::Groth16
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();
        input_builder.write_buf(&input.start_state.as_ssz_bytes())?;
        input_builder.write_buf(&input.blocks.as_ssz_bytes())?;
        input_builder.write_buf(&input.parent.as_ssz_bytes())?;
        input_builder.write_buf(&input.da_state_diff_bytes)?;
        input_builder.write_buf(&input.bridge_params.as_ssz_bytes())?;
        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        CheckpointClaim::from_ssz_bytes(public_values.as_bytes())
            .map_err(|e| ZkVmError::Other(e.to_string()))
    }
}

impl CheckpointProgram {
    pub fn native_host() -> NativeHost {
        NativeHost::new(test_signing_key(), process_ol_stf)
    }

    /// Predicate key matching the signing key the native host uses, for wiring into
    /// functional-test params so the resulting witness verifies under `Bip340Schnorr`.
    pub fn test_predicate_key() -> PredicateKey {
        let pk = test_signing_key().verifying_key().to_bytes().to_vec();
        PredicateKey::new(PredicateTypeId::Bip340Schnorr, pk)
    }

    /// Executes the checkpoint program using the native host for testing.
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        // Get the native host and delegate to the trait's execute method
        let host = Self::native_host();
        let summary = <Self as ZkVmProgram>::execute(input, &host)?;
        <Self as ZkVmProgram>::process_output::<NativeHost>(summary.public_values())
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use ssz::Encode;
    use strata_acct_types::{BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount};
    use strata_asm_proto_checkpoint_types::TerminalHeaderComplement;
    use strata_bridge_params::BridgeParams;
    use strata_codec::encode_to_vec;
    use strata_crypto::hash;
    use strata_da_framework::DaCounter;
    use strata_identifiers::{
        AccountSerial, Buf32, Buf64, L1BlockId, L1Height, SubjectId, WtxidsRoot,
    };
    use strata_ledger_types::IStateAccessor;
    use strata_ol_chain_types_new::{
        AsmManifest, MAX_SEALING_MANIFEST_COUNT, OLBlock, SignedOLBlockHeader,
    };
    use strata_ol_da::{GlobalStateDiff, LedgerDiff, OLDaPayloadV1, StateDiff};
    use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};
    use strata_ol_stf::{
        execute_block_batch_predrain,
        test_utils::{
            OLStfFixture, TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID, build_empty_chain,
            make_account_id, make_deposit_log_for_account, make_empty_manifest, make_genesis_state,
            make_withdrawal_payload,
        },
    };

    use crate::program::{CheckpointProgram, CheckpointProverInput};

    const CAPACITY_PROOF_BLOCKS: usize = 64;
    const CAPACITY_TRANSFER_AMOUNT_SATS: u64 = 1;
    const CAPACITY_WITHDRAWAL_AMOUNT_SATS: u64 = 100_000_000;
    // Mirrors `MAX_LOGS_PER_MANIFEST` from the ASM manifest SSZ type.
    const MAX_ASM_LOGS_PER_MANIFEST: usize = 1024;
    // Mirrors `TxEffects.MAX_MESSAGES` from `crates/acct-types/ssz/effects.ssz`.
    const TX_EFFECTS_MAX_MESSAGES: usize = 255;
    const MAX_OL_LOGS_PER_CAPACITY_TX: usize = TX_EFFECTS_MAX_MESSAGES + 1;
    const NEAR_HARD_OL_LOG_TARGET: usize = 16_128;
    const NEAR_PENDING_ASM_LOG_CAP_LOGS_PER_MANIFEST: usize = 128;

    fn prepare_input() -> CheckpointProverInput {
        const SLOTS_PER_EPOCH: u64 = 9;

        let mut state = make_genesis_state();
        let mut blocks = build_empty_chain(&mut state, 10, SLOTS_PER_EPOCH).unwrap();
        let parent = blocks.remove(0).into_header();

        // Start state is after the genesis block
        let mut start_state = make_genesis_state();
        let _ = build_empty_chain(&mut start_state, 1, SLOTS_PER_EPOCH).unwrap();

        let blocks: Vec<OLBlock> = blocks
            .into_iter()
            .map(|b| {
                OLBlock::new(
                    SignedOLBlockHeader::new(b.header().clone(), Buf64::zero()),
                    b.body().clone(),
                )
            })
            .collect();

        let terminal_header = blocks.last().expect("non-empty block list").header();
        let slot_delta = terminal_header.slot() - start_state.cur_slot();
        let slot_delta_u16 =
            u16::try_from(slot_delta).expect("slot delta exceeds u16::MAX; epoch too long");
        let da_diff = StateDiff::new(
            GlobalStateDiff::new(
                DaCounter::new_changed(slot_delta_u16),
                DaCounter::new_unchanged(),
            ),
            LedgerDiff::default(),
        );
        let da_state_diff_bytes =
            encode_to_vec(&OLDaPayloadV1::new(da_diff)).expect("encode DA payload");

        CheckpointProverInput {
            start_state: start_state.state().clone(),
            blocks,
            parent,
            da_state_diff_bytes,
            bridge_params: BridgeParams::default(),
        }
    }

    fn buf32_from_l1_height(height: L1Height, domain: u8) -> Buf32 {
        let mut bytes = [domain; 32];
        let height_bytes = height.to_le_bytes();
        bytes[..height_bytes.len()].copy_from_slice(&height_bytes);
        Buf32::from(bytes)
    }

    fn make_deposit_manifest(
        height: L1Height,
        account_serial: AccountSerial,
        dest_subject: SubjectId,
        amount: BitcoinAmount,
        log_count: usize,
    ) -> AsmManifest {
        assert!(
            log_count <= MAX_ASM_LOGS_PER_MANIFEST,
            "deposit manifest log count exceeds MAX_LOGS_PER_MANIFEST"
        );
        let deposit_log = make_deposit_log_for_account(account_serial, dest_subject, amount);
        AsmManifest::new(
            height,
            L1BlockId::from(buf32_from_l1_height(height, 0xa1)),
            WtxidsRoot::from(buf32_from_l1_height(height, 0xb2)),
            vec![deposit_log; log_count],
        )
        .expect("deposit manifest should be valid")
    }

    fn withdrawal_counts_by_block(ol_log_target: Option<usize>) -> Vec<Option<usize>> {
        (1..=CAPACITY_PROOF_BLOCKS)
            .map(|block_idx| match ol_log_target {
                Some(target) => {
                    let prior_capacity = (block_idx - 1) * MAX_OL_LOGS_PER_CAPACITY_TX;
                    if prior_capacity >= target {
                        None
                    } else {
                        let logs_this_tx =
                            (target - prior_capacity).min(MAX_OL_LOGS_PER_CAPACITY_TX);
                        Some(logs_this_tx.saturating_sub(1))
                    }
                }
                None => Some(1),
            })
            .collect()
    }

    fn prepare_input_with_terminal_manifests(manifest_count: usize) -> CheckpointProverInput {
        prepare_input_with_terminal_manifests_and_targets(manifest_count, None, 1)
    }

    fn prepare_input_with_terminal_manifests_and_targets(
        manifest_count: usize,
        ol_log_target: Option<usize>,
        asm_logs_per_manifest: usize,
    ) -> CheckpointProverInput {
        assert!(
            manifest_count <= MAX_SEALING_MANIFEST_COUNT as usize,
            "manifest count exceeds MAX_SEALING_MANIFEST_COUNT"
        );
        assert!(
            asm_logs_per_manifest > 0,
            "asm logs per manifest must be greater than zero"
        );
        assert!(
            asm_logs_per_manifest <= MAX_ASM_LOGS_PER_MANIFEST,
            "asm logs per manifest exceeds MAX_LOGS_PER_MANIFEST"
        );
        let max_representable = CAPACITY_PROOF_BLOCKS * MAX_OL_LOGS_PER_CAPACITY_TX;
        assert!(
            ol_log_target.unwrap_or_default() <= max_representable,
            "ol log target exceeds max representable OL logs"
        );

        let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
        let recipient_id = make_account_id(TEST_RECIPIENT_ID);
        let dest_subject = SubjectId::from([42u8; 32]);
        let deposit_amount = BitcoinAmount::from_sat(CAPACITY_WITHDRAWAL_AMOUNT_SATS);
        let withdrawal_counts = withdrawal_counts_by_block(ol_log_target);
        let withdrawal_count = withdrawal_counts
            .iter()
            .map(|maybe_count| maybe_count.unwrap_or_default())
            .sum::<usize>();
        let transfer_count = match ol_log_target {
            Some(_) => withdrawal_counts
                .iter()
                .filter(|maybe_count| matches!(maybe_count, Some(0)))
                .count(),
            None => CAPACITY_PROOF_BLOCKS,
        };
        let total_withdrawal_sats = (withdrawal_count as u64)
            .checked_mul(CAPACITY_WITHDRAWAL_AMOUNT_SATS)
            .expect("withdrawal capacity balance should fit in u64");
        let total_transfer_sats = (transfer_count as u64)
            .checked_mul(CAPACITY_TRANSFER_AMOUNT_SATS)
            .expect("transfer capacity balance should fit in u64");
        let fixture_builder = OLStfFixture::builder();
        let snark_acct_serial = fixture_builder.next_account_serial();
        let initial_balance = total_withdrawal_sats
            .checked_add(total_transfer_sats)
            .expect("capacity balance should fit in u64");
        let mut fixture = fixture_builder
            .with_genesis_snark_account(snark_acct_id, |acct| {
                acct.with_balance(BitcoinAmount::from_sat(initial_balance))
            })
            .with_genesis_empty_account(recipient_id)
            .with_genesis_manifest(make_empty_manifest(1, 0))
            .execute_genesis();

        let parent = fixture.last_completed_block().header().clone();
        let start_state = fixture.state().state().clone();
        let mut blocks = Vec::with_capacity(CAPACITY_PROOF_BLOCKS);

        for block_idx in 1..=CAPACITY_PROOF_BLOCKS {
            let manifests = if block_idx == CAPACITY_PROOF_BLOCKS {
                let first_manifest_height = fixture.state().last_l1_height() + 1;
                (0..manifest_count)
                    .map(|offset| {
                        make_deposit_manifest(
                            first_manifest_height + offset as L1Height,
                            snark_acct_serial,
                            dest_subject,
                            deposit_amount,
                            asm_logs_per_manifest,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };

            let mut block_builder = fixture.child_block();
            if let Some(withdrawal_count) = withdrawal_counts[block_idx - 1] {
                block_builder = block_builder.with_sau(snark_acct_id, |mut sau| {
                    if ol_log_target.is_none() || withdrawal_count == 0 {
                        sau = sau.transfer(
                            recipient_id,
                            BitcoinAmount::from_sat(CAPACITY_TRANSFER_AMOUNT_SATS),
                        );
                    }

                    for msg_idx in 0..withdrawal_count {
                        let withdrawal_dest = match ol_log_target {
                            Some(_) => {
                                format!("bc1qcapacity{block_idx:02}{msg_idx:03}").into_bytes()
                            }
                            None => format!("bc1qcapacity{block_idx:02}").into_bytes(),
                        };
                        sau = sau.output_message(
                            BRIDGE_GATEWAY_ACCT_ID,
                            BitcoinAmount::from_sat(CAPACITY_WITHDRAWAL_AMOUNT_SATS),
                            make_withdrawal_payload(withdrawal_dest),
                        );
                    }

                    sau
                });
            }
            if !manifests.is_empty() {
                block_builder = block_builder.with_manifests(manifests);
            }
            if block_idx == CAPACITY_PROOF_BLOCKS {
                block_builder = block_builder.terminal();
            }

            let block = block_builder.execute().completed_block().clone();
            blocks.push(OLBlock::new(
                SignedOLBlockHeader::new(block.header().clone(), Buf64::zero()),
                block.body().clone(),
            ));
        }

        let mut da_state = DaAccumulatingState::new(MemoryStateBaseLayer::new(start_state.clone()));
        execute_block_batch_predrain(&mut da_state, &blocks, &parent, BridgeParams::default())
            .expect("checkpoint input should replay for DA predrain");
        let da_state_diff_bytes = da_state
            .take_completed_epoch_da_blob()
            .expect("DA blob extraction should succeed")
            .expect("terminal checkpoint input should produce a DA blob");

        CheckpointProverInput {
            start_state,
            blocks,
            parent,
            da_state_diff_bytes,
            bridge_params: BridgeParams::default(),
        }
    }

    #[test]
    fn test_statements_success() {
        let input = prepare_input();

        let claim = CheckpointProgram::execute(&input).unwrap();

        assert_eq!(
            *claim.l2_range().start().blkid(),
            input.parent.compute_blkid()
        );

        assert_eq!(
            *claim.l2_range().end().blkid(),
            input.blocks.last().unwrap().header().compute_blkid()
        );

        assert_eq!(
            *claim.state_diff_hash(),
            hash::raw(&input.da_state_diff_bytes).into()
        );
        let terminal_header = input.blocks.last().expect("non-empty block list").header();
        let terminal_header_complement = TerminalHeaderComplement::new(
            terminal_header.timestamp(),
            *terminal_header.parent_blkid(),
            *terminal_header.body_root(),
            *terminal_header.logs_root(),
        );
        assert_eq!(
            *claim.terminal_header_complement_hash(),
            terminal_header_complement.compute_hash()
        );
    }

    #[test]
    #[ignore = "manual native capacity check for candidate terminal manifest counts"]
    fn test_native_execution_candidate_terminal_manifest_counts() {
        let mut candidates = vec![256, 512, 1024, MAX_SEALING_MANIFEST_COUNT as usize];
        candidates.retain(|count| *count <= MAX_SEALING_MANIFEST_COUNT as usize);
        candidates.sort_unstable();
        candidates.dedup();

        for manifest_count in candidates {
            let input = prepare_input_with_terminal_manifests(manifest_count);
            let encoded_blocks_len = input.blocks.as_ssz_bytes().len();
            let claim = CheckpointProgram::execute(&input).unwrap_or_else(|e| {
                panic!("native checkpoint execution failed for {manifest_count} manifests: {e}")
            });

            assert_eq!(
                *claim.l2_range().end().blkid(),
                input.blocks.last().unwrap().header().compute_blkid(),
                "claim terminal block mismatch for {manifest_count} manifests"
            );

            eprintln!(
                "native checkpoint execution succeeded: manifests={manifest_count}, encoded_blocks_bytes={encoded_blocks_len}"
            );
        }
    }

    #[test]
    #[ignore = "manual native capacity check for 2048 manifests with near-max OL logs"]
    fn test_native_execution_2048_terminal_manifests_with_near_max_ol_logs() {
        let manifest_count = MAX_SEALING_MANIFEST_COUNT as usize;
        let input = prepare_input_with_terminal_manifests_and_targets(
            manifest_count,
            Some(NEAR_HARD_OL_LOG_TARGET),
            1,
        );
        let encoded_blocks_len = input.blocks.as_ssz_bytes().len();
        let claim = CheckpointProgram::execute(&input).unwrap_or_else(|e| {
            panic!(
                "native checkpoint execution failed for {manifest_count} manifests and {NEAR_HARD_OL_LOG_TARGET} OL logs: {e}"
            )
        });

        assert_eq!(
            *claim.l2_range().end().blkid(),
            input.blocks.last().unwrap().header().compute_blkid(),
            "claim terminal block mismatch for {manifest_count} manifests and {NEAR_HARD_OL_LOG_TARGET} OL logs"
        );

        eprintln!(
            "native checkpoint execution succeeded: manifests={manifest_count}, ol_log_target={NEAR_HARD_OL_LOG_TARGET}, encoded_blocks_bytes={encoded_blocks_len}"
        );
    }

    #[test]
    #[ignore = "manual native capacity check for 2048 manifests with near-cap ASM logs per manifest"]
    fn test_native_execution_2048_terminal_manifests_with_near_cap_asm_logs() {
        let manifest_count = MAX_SEALING_MANIFEST_COUNT as usize;
        let input = prepare_input_with_terminal_manifests_and_targets(
            manifest_count,
            None,
            NEAR_PENDING_ASM_LOG_CAP_LOGS_PER_MANIFEST,
        );
        let encoded_blocks_len = input.blocks.as_ssz_bytes().len();
        let claim = CheckpointProgram::execute(&input).unwrap_or_else(|e| {
            panic!(
                "native checkpoint execution failed for {manifest_count} manifests and {NEAR_PENDING_ASM_LOG_CAP_LOGS_PER_MANIFEST} ASM logs per manifest: {e}"
            )
        });

        assert_eq!(
            *claim.l2_range().end().blkid(),
            input.blocks.last().unwrap().header().compute_blkid(),
            "claim terminal block mismatch for {manifest_count} manifests and {NEAR_PENDING_ASM_LOG_CAP_LOGS_PER_MANIFEST} ASM logs per manifest"
        );

        eprintln!(
            "native checkpoint execution succeeded: manifests={manifest_count}, asm_logs_per_manifest={NEAR_PENDING_ASM_LOG_CAP_LOGS_PER_MANIFEST}, encoded_blocks_bytes={encoded_blocks_len}"
        );
    }

    #[test]
    fn test_statements_fail_on_invalid_da_payload_encoding() {
        let mut input = prepare_input();
        input.da_state_diff_bytes = vec![1, 2, 3, 4];

        let panic_res = catch_unwind(|| CheckpointProgram::execute(&input));
        assert!(
            panic_res.is_err(),
            "invalid DA payload encoding must panic in statement verification"
        );
    }

    #[test]
    fn test_statements_fail_on_da_diff_mismatch() {
        let mut input = prepare_input();
        let terminal_header = input.blocks.last().expect("non-empty block list").header();
        let start_state_layer = MemoryStateBaseLayer::new(input.start_state.clone());
        let slot_delta = terminal_header.slot() - start_state_layer.cur_slot();
        let bad_delta = u16::try_from(slot_delta.saturating_sub(1))
            .expect("slot delta exceeds u16::MAX; epoch too long");
        let bad_da_diff = StateDiff::new(
            GlobalStateDiff::new(
                DaCounter::new_changed(bad_delta),
                DaCounter::new_unchanged(),
            ),
            LedgerDiff::default(),
        );
        input.da_state_diff_bytes =
            encode_to_vec(&OLDaPayloadV1::new(bad_da_diff)).expect("encode bad DA payload");

        let panic_res = catch_unwind(|| CheckpointProgram::execute(&input));
        assert!(
            panic_res.is_err(),
            "mismatched DA witness must panic in statement verification"
        );
    }
}
