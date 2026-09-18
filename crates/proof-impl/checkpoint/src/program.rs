use k256::schnorr::SigningKey;
use ssz::{Decode, Encode};
use strata_asm_checkpoint_types::CheckpointClaim;
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types_v1::OLStateV1;
use strata_predicate::{PredicateKey, PredicateTypeId};
use zkaleido::{PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::NativeHost;

use crate::statements::process_ol_stf;

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x01u8; 32]).expect("valid test signing key")
}

#[derive(Debug)]
pub struct CheckpointProverInput {
    pub start_state: OLStateV1,
    pub blocks: Vec<OLBlockV1>,
    pub parent: OLBlockHeaderV1,
    pub da_state_diff_bytes: Vec<u8>,
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
    pub fn native_host(runtime_params: OLRuntimeParams) -> NativeHost {
        NativeHost::new(test_signing_key(), move |zkvm| {
            process_ol_stf(zkvm, &runtime_params)
        })
    }

    /// Predicate key matching the signing key the native host uses, for wiring into
    /// functional-test params so the resulting witness verifies under `Bip340Schnorr`.
    pub fn test_predicate_key() -> PredicateKey {
        let pk = test_signing_key().verifying_key().to_bytes().to_vec();
        PredicateKey::try_new(PredicateTypeId::Bip340Schnorr, pk)
            .expect("predicate condition must fit within the maximum length")
    }

    /// Executes the checkpoint program using the native host.
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
        runtime_params: OLRuntimeParams,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        // Get the native host and delegate to the trait's execute method
        let host = Self::native_host(runtime_params);
        let summary = <Self as ZkVmProgram>::execute(input, &host)?;
        <Self as ZkVmProgram>::process_output::<NativeHost>(summary.public_values())
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use strata_acct_types::{BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount};
    use strata_asm_checkpoint_types::TerminalHeaderComplement;
    use strata_codec::encode_to_vec;
    use strata_crypto::hash;
    use strata_da_framework::DaCounter;
    use strata_identifiers::Buf64;
    use strata_ol_chain_types_v1::{
        OLBlockBodyV1, OLBlockV1, OLTxSegmentV1, SignedOLBlockHeaderV1,
    };
    use strata_ol_da_types_v1::{GlobalStateDiffV1, LedgerDiffV1, OLDaPayloadV1, OLStateDiffV1};
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::{EpochLogBudgetError, ExecError, IStateAccessor};
    use strata_ol_stf_v1::test_utils::{
        OLStfFixture, SnarkUpdateBuilder, assert_verification_fails_with, build_empty_chain,
        make_account_id, make_genesis_state, make_proof, make_state_root, make_withdrawal_payload,
        tamper_body_root, to_ol_block,
    };

    use crate::{
        process_ol_stf_core,
        program::{CheckpointProgram, CheckpointProverInput},
    };

    #[test]
    fn test_verification_and_checkpoint_proof_reject_epoch_log_overflow() {
        let account = make_account_id(100);
        let mut fixture = OLStfFixture::builder()
            .with_genesis_snark_account(account, |acct| {
                acct.with_balance(BitcoinAmount::try_from(18_000_000_000).unwrap())
            })
            .execute_genesis();
        let start_state = fixture.state().state().clone();
        let parent = fixture.last_completed_block().header().clone();
        let withdrawal_tx = |fixture: &OLStfFixture| {
            let mut builder =
                SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(account).clone());
            for _ in 0..90 {
                builder = builder.with_output_message(
                    BRIDGE_GATEWAY_ACCT_ID,
                    100_000_000,
                    make_withdrawal_payload(vec![0; 81]),
                );
            }
            builder.build(account, make_state_root(2), make_proof(1))
        };
        let first_tx = withdrawal_tx(&fixture);
        let first = fixture.child_block().with_tx(first_tx).execute();
        let next_tx = withdrawal_tx(&fixture);
        let mut verify_state = fixture.state().clone();
        let terminal = fixture.child_block().terminal().execute();
        let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![next_tx]).unwrap());
        let header = tamper_body_root(
            terminal.completed_block().header(),
            body.compute_hash_commitment(),
        );

        assert_verification_fails_with(
            &mut verify_state,
            &header,
            Some(first.completed_block().header().clone()),
            &body,
            |error| {
                matches!(
                    error.base(),
                    ExecError::EpochLogBudget(EpochLogBudgetError::LogPayloadBytes {
                        actual: 17_120,
                        limit: 16_384
                    })
                )
            },
        );
        let blocks = vec![
            to_ol_block(first.completed_block()),
            OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body),
        ];
        // Execution must reject before the proof program reaches DA verification.
        let error = catch_unwind(|| {
            process_ol_stf_core(
                start_state,
                blocks,
                parent,
                vec![],
                &OLRuntimeParams::test_default(),
            )
        })
        .unwrap_err();
        let message = error
            .downcast_ref::<String>()
            .expect("proof execution panic message");
        assert!(
            message.contains("EpochLogBudget(LogPayloadBytes { actual: 17120, limit: 16384 })"),
            "{message}"
        );
    }

    fn prepare_input() -> CheckpointProverInput {
        const SLOTS_PER_EPOCH: u64 = 9;

        let mut state = make_genesis_state();
        let mut blocks = build_empty_chain(&mut state, 10, SLOTS_PER_EPOCH).unwrap();
        let parent = blocks.remove(0).into_header();

        // Start state is after the genesis block
        let mut start_state = make_genesis_state();
        let _ = build_empty_chain(&mut start_state, 1, SLOTS_PER_EPOCH).unwrap();

        let blocks: Vec<OLBlockV1> = blocks
            .into_iter()
            .map(|b| {
                OLBlockV1::new(
                    SignedOLBlockHeaderV1::new(b.header().clone(), Buf64::zero()),
                    b.body().clone(),
                )
            })
            .collect();

        let terminal_header = blocks.last().expect("non-empty block list").header();
        let slot_delta = terminal_header.slot() - start_state.cur_slot();
        let slot_delta_u16 =
            u16::try_from(slot_delta).expect("slot delta exceeds u16::MAX; epoch too long");
        let da_diff = OLStateDiffV1::new(
            GlobalStateDiffV1::new(
                DaCounter::new_changed(slot_delta_u16),
                DaCounter::new_unchanged(),
            ),
            LedgerDiffV1::default(),
        );
        let da_state_diff_bytes =
            encode_to_vec(&OLDaPayloadV1::new(da_diff)).expect("encode DA payload");

        CheckpointProverInput {
            start_state: start_state.state().clone(),
            blocks,
            parent,
            da_state_diff_bytes,
        }
    }

    #[test]
    fn test_statements_success() {
        let input = prepare_input();

        let claim = CheckpointProgram::execute(&input, OLRuntimeParams::test_default()).unwrap();

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
    fn test_statements_fail_on_invalid_da_payload_encoding() {
        let mut input = prepare_input();
        input.da_state_diff_bytes = vec![1, 2, 3, 4];

        let panic_res =
            catch_unwind(|| CheckpointProgram::execute(&input, OLRuntimeParams::test_default()));
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
        let bad_da_diff = OLStateDiffV1::new(
            GlobalStateDiffV1::new(
                DaCounter::new_changed(bad_delta),
                DaCounter::new_unchanged(),
            ),
            LedgerDiffV1::default(),
        );
        input.da_state_diff_bytes =
            encode_to_vec(&OLDaPayloadV1::new(bad_da_diff)).expect("encode bad DA payload");

        let panic_res =
            catch_unwind(|| CheckpointProgram::execute(&input, OLRuntimeParams::test_default()));
        assert!(
            panic_res.is_err(),
            "mismatched DA witness must panic in statement verification"
        );
    }
}
