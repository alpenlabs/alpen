use moho_types::{ExportState, InnerStateCommitment, StateReference};
use strata_asm_common::{AnchorState, AsmSpec};
use strata_asm_logs::{AsmStfUpdate, NewExportEntry};
use strata_asm_spec::StrataAsmSpec;
use strata_asm_stf::{AsmStfInput, AsmStfOutput, compute_asm_transition, group_txs_by_subprotocol};
use strata_primitives::hash::compute_borsh_hash;
use zkaleido::VerifyingKey;

use crate::{input::AsmStepInput, traits::MohoProgram};

#[derive(Debug)]
pub struct AsmStfProgram;

impl MohoProgram for AsmStfProgram {
    type State = AnchorState;

    type StepInput = AsmStepInput;

    type Spec = StrataAsmSpec;

    type StepOutput = AsmStfOutput;

    fn compute_input_reference(input: &AsmStepInput) -> StateReference {
        input.compute_ref()
    }

    fn extract_prev_reference(input: &Self::StepInput) -> StateReference {
        input.compute_prev_ref()
    }

    fn compute_state_commitment(state: &AnchorState) -> InnerStateCommitment {
        InnerStateCommitment::new(compute_borsh_hash(state).into())
    }

    fn process_transition(
        pre_state: &AnchorState,
        spec: &StrataAsmSpec,
        input: &AsmStepInput,
    ) -> AsmStfOutput {
        // 1. Validate the input
        assert!(input.validate_block());

        // 2. Restructure the raw input to be formatted according to what we want.
        let protocol_txs = group_txs_by_subprotocol(spec.magic_bytes(), &input.block.0.txdata);
        let stf_input = AsmStfInput {
            protocol_txs,
            header: &input.block.0.header,
            aux_responses: &input.aux_inputs,
        };

        // 3. Actually invoke the ASM state transition function.
        compute_asm_transition(spec, pre_state, stf_input).expect("asm: compute transition")
    }

    fn extract_post_state(output: &Self::StepOutput) -> &Self::State {
        &output.state
    }

    fn extract_next_vk(output: &Self::StepOutput) -> Option<VerifyingKey> {
        // Iterate through each AsmLog; if we find an AsmStfUpdate, grab its vk and return it.
        output.logs.iter().find_map(|log| {
            log.try_into_log::<AsmStfUpdate>()
                .ok()
                .map(|update| update.new_vk.clone())
        })
    }

    fn compute_export_state(export_state: ExportState, output: &Self::StepOutput) -> ExportState {
        // Iterate through each AsmLog; if we find an NewExportEntry, add it to ExportState
        let mut new_export_state = export_state;
        for log in &output.logs {
            if let Ok(export) = log.try_into_log::<NewExportEntry>() {
                new_export_state.add_entry(export.container_id, export.entry_data.clone());
            }
        }
        new_export_state
    }
}
