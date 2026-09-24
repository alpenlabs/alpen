use k256::schnorr::SigningKey;
use ssz::{Decode, Encode};
use strata_asm_checkpoint_types::CheckpointClaim;
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_stf::OLSpecId;
use strata_predicate::{PredicateKey, PredicateTypeId};
use zkaleido::{PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::NativeHost;

use crate::statements::process_ol_stf;

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x01u8; 32]).expect("valid test signing key")
}

#[derive(Debug)]
pub struct CheckpointProverInput {
    /// Terminal state of the previous epoch, with its spec versions.
    pub start_state: OLStateContainer,
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
        input_builder.write_serde(&input.start_state)?;
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
    /// Returns a native host that proves `spec`'s rules.
    pub fn native_host(spec: OLSpecId, runtime_params: OLRuntimeParams) -> NativeHost {
        NativeHost::new(test_signing_key(), move |zkvm| {
            process_ol_stf(zkvm, spec, &runtime_params)
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
        spec: OLSpecId,
        runtime_params: OLRuntimeParams,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        // Get the native host and delegate to the trait's execute method
        let host = Self::native_host(spec, runtime_params);
        let summary = <Self as ZkVmProgram>::execute(input, &host)?;
        <Self as ZkVmProgram>::process_output::<NativeHost>(summary.public_values())
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use strata_asm_checkpoint_types::TerminalHeaderComplement;
    use strata_asm_logs::CheckpointPredicateEnacted;
    use strata_asm_manifest_types::AsmLogEntry;
    use strata_codec::encode_to_vec;
    use strata_crypto::hash;
    use strata_da_framework::DaCounter;
    use strata_identifiers::Buf64;
    use strata_ol_chain_types_v1::{
        OLAsmManifestContainerV1, OLBlockBodyV1, OLBlockV1, OLTxSegmentV1, SignedOLBlockHeaderV1,
    };
    use strata_ol_da_types_v1::{GlobalStateDiffV1, LedgerDiffV1, OLDaPayloadV1, OLStateDiffV1};
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::IStateAccessor;
    use strata_ol_stf::OLSpecId;
    use strata_ol_stf_v1::test_utils::{
        FixtureAsmManifestBuilder, OLStfFixture, build_empty_chain, make_genesis_state,
    };
    use strata_predicate::PredicateKey;

    use crate::program::{CheckpointProgram, CheckpointProverInput};

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
            start_state: start_state.into_container(),
            blocks,
            parent,
            da_state_diff_bytes,
        }
    }

    fn prepare_boundary_input() -> CheckpointProverInput {
        let mut fixture = OLStfFixture::builder().execute_genesis();
        let start_state = fixture.state().to_container();
        let parent = fixture.parent_header().clone();
        let log = AsmLogEntry::from_log(&CheckpointPredicateEnacted::new(
            PredicateKey::always_accept(),
        ))
        .unwrap();
        let manifest = FixtureAsmManifestBuilder::new_at_height(1)
            .with_log(log)
            .build();
        let outcome = fixture
            .child_block()
            .with_manifest(manifest)
            .terminal()
            .execute();
        let block = outcome.completed_block();
        let diff = OLStateDiffV1::new(
            GlobalStateDiffV1::new(DaCounter::new_changed(1), DaCounter::new_unchanged()),
            LedgerDiffV1::default(),
        );
        CheckpointProverInput {
            start_state,
            blocks: vec![OLBlockV1::new(
                SignedOLBlockHeaderV1::new(block.header().clone(), Buf64::zero()),
                block.body().clone(),
            )],
            parent,
            da_state_diff_bytes: encode_to_vec(&OLDaPayloadV1::new(diff)).unwrap(),
        }
    }

    #[test]
    fn test_statements_accept_epoch_ending_at_predicate_boundary() {
        CheckpointProgram::execute(
            &prepare_boundary_input(),
            OLSpecId::V1,
            OLRuntimeParams::test_default(),
        )
        .unwrap();
    }

    #[test]
    #[should_panic(expected = "CheckpointPredicateBoundaryNotLast")]
    fn test_statements_reject_epoch_straddling_predicate_boundary() {
        let mut input = prepare_boundary_input();
        let block = &input.blocks[0];
        let mut manifests = block.body().manifests().unwrap().manifests().to_vec();
        manifests.push(FixtureAsmManifestBuilder::new_at_height(2).build());
        let body = OLBlockBodyV1::new(
            OLTxSegmentV1::new(vec![]).unwrap(),
            Some(OLAsmManifestContainerV1::new(manifests).unwrap()),
        );
        let mut header = block.header().clone();
        header.body_root = body.compute_hash_commitment();
        input.blocks[0] = OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body);
        // Require the boundary error specifically, before any commitment mismatch.
        let _ = CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default());
    }

    #[test]
    #[should_panic(expected = "CheckpointPredicateBoundaryNonterminal")]
    fn test_statements_reject_boundary_in_earlier_nonterminal_block() {
        let mut input = prepare_input();
        let boundary = prepare_boundary_input();
        let body = boundary.blocks[0].body().clone();
        let mut header = input.blocks[0].header().clone();
        header.body_root = body.compute_hash_commitment();
        input.blocks[0] = OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body);
        let _ = CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default());
    }

    #[test]
    fn test_statements_success() {
        let input = prepare_input();

        let claim =
            CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default())
                .unwrap();

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

        let panic_res = catch_unwind(|| {
            CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default())
        });
        assert!(
            panic_res.is_err(),
            "invalid DA payload encoding must panic in statement verification"
        );
    }

    #[test]
    fn test_statements_fail_on_da_diff_mismatch() {
        let mut input = prepare_input();
        let terminal_header = input.blocks.last().expect("non-empty block list").header();
        let start_state_layer = MemoryStateBaseLayer::from_container(input.start_state.clone());
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

        let panic_res = catch_unwind(|| {
            CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default())
        });
        assert!(
            panic_res.is_err(),
            "mismatched DA witness must panic in statement verification"
        );
    }
}
