use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use ssz::{Decode, Encode};
use strata_checkpoint_types_ssz::CheckpointClaim;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_types::OLState;
use zkaleido::{PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::statements::process_ol_stf;

#[derive(Debug)]
pub struct CheckpointProverInput {
    pub start_state: OLState,
    pub blocks: Vec<OLBlock>,
    pub parent: OLBlockHeader,
}

#[derive(Debug)]
pub struct CheckpointProgram;

impl ZkVmProgram for CheckpointProgram {
    type Input = CheckpointProverInput;
    type Output = CheckpointClaim;

    fn name() -> String {
        "OL Checkpoint".to_string()
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
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_ol_stf(zkvm);
                }))
                .map_err(|_| ZkVmError::ExecutionError(Self::name()))?;
                Ok(())
            })),
        }
    }

    // Add this new convenience method
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
    use strata_identifiers::Buf64;
    use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
    use strata_ol_state_types::OLState;
    use strata_ol_stf::test_utils::build_empty_chain;

    use crate::program::{CheckpointProgram, CheckpointProverInput};

    fn prepare_input() -> CheckpointProverInput {
        const SLOTS_PER_EPOCH: u64 = 100;

        let mut state = OLState::new_genesis();
        let mut blocks = build_empty_chain(&mut state, 10, SLOTS_PER_EPOCH).unwrap();
        let parent = blocks.remove(0).into_header();

        // Start state is after the genesis block
        let mut start_state = OLState::new_genesis();
        let _ = build_empty_chain(&mut start_state, 1, SLOTS_PER_EPOCH).unwrap();

        let blocks = blocks
            .into_iter()
            .map(|b| {
                OLBlock::new(
                    SignedOLBlockHeader::new(b.header().clone(), Buf64::zero()),
                    b.body().clone(),
                )
            })
            .collect();

        CheckpointProverInput {
            start_state,
            blocks,
            parent,
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
    }
}
