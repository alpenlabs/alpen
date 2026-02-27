use ssz::{Decode, Encode};
use strata_checkpoint_types_ssz::CheckpointClaim;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_types::OLState;
use zkaleido::{PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::NativeHost;

use crate::statements::process_ol_stf;

#[derive(Debug)]
pub struct CheckpointProverInput {
    pub start_state: OLState,
    pub blocks: Vec<OLBlock>,
    pub parent: OLBlockHeader,
    pub da_state_diff_bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct CheckpointProgram;

impl ZkVmProgram for CheckpointProgram {
    type Input = CheckpointProverInput;
    type Output = CheckpointClaim;

    fn name() -> String {
        "Checkpoint New".to_string()
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
    pub fn native_host() -> NativeHost {
        NativeHost::new(process_ol_stf)
    }

    /// Executes the checkpoint program using the native host for testing.
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        // Get the native host and delegate to the trait's execute method
        let host = Self::native_host();
        <Self as ZkVmProgram>::execute(input, &host)
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use strata_checkpoint_types_ssz::TerminalHeaderComplement;
    use strata_codec::encode_to_vec;
    use strata_crypto::hash;
    use strata_da_framework::DaCounter;
    use strata_identifiers::Buf64;
    use strata_ledger_types::IStateAccessor;
    use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
    use strata_ol_da::{GlobalStateDiff, LedgerDiff, OLDaPayloadV1, StateDiff};
    use strata_ol_stf::test_utils::{build_empty_chain, create_test_genesis_state};

    use crate::program::{CheckpointProgram, CheckpointProverInput};

    fn prepare_input() -> CheckpointProverInput {
        const SLOTS_PER_EPOCH: u64 = 9;

        let mut state = create_test_genesis_state();
        let mut blocks = build_empty_chain(&mut state, 10, SLOTS_PER_EPOCH).unwrap();
        let parent = blocks.remove(0).into_header();

        // Start state is after the genesis block
        let mut start_state = create_test_genesis_state();
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
            GlobalStateDiff::new(DaCounter::new_changed(slot_delta_u16)),
            LedgerDiff::default(),
        );
        let da_state_diff_bytes =
            encode_to_vec(&OLDaPayloadV1::new(da_diff)).expect("encode DA payload");

        CheckpointProverInput {
            start_state,
            blocks,
            parent,
            da_state_diff_bytes,
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
        let terminal_header_complement =
            TerminalHeaderComplement::from_full_header(terminal_header);
        assert_eq!(
            *claim.terminal_header_complement_hash(),
            terminal_header_complement.compute_hash()
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
    fn test_statements_fail_on_da_preseal_mismatch() {
        let mut input = prepare_input();
        let terminal_header = input.blocks.last().expect("non-empty block list").header();
        let slot_delta = terminal_header.slot() - input.start_state.cur_slot();
        let bad_delta = u16::try_from(slot_delta.saturating_sub(1))
            .expect("slot delta exceeds u16::MAX; epoch too long");
        let bad_da_diff = StateDiff::new(
            GlobalStateDiff::new(DaCounter::new_changed(bad_delta)),
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
