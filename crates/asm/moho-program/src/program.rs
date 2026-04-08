use moho_types::{ExportState, InnerStateCommitment, StateReference};
use strata_asm_common::AnchorState;
use strata_asm_logs::{AsmStfUpdate, NewExportEntry};
use strata_asm_spec::StrataAsmSpec;
use strata_asm_stf::{AsmStfOutput, compute_asm_transition};
use strata_crypto::hash::compute_borsh_hash;
use strata_predicate::PredicateKey;

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
        // The new ASM STF entry point internally validates block integrity,
        // groups txs by subprotocol via `pre_state.magic()`, and computes the
        // wtxids root. Callers just provide block, aux data, and an optional
        // coinbase inclusion proof. Pass `None` here for now — alpen's runner
        // path doesn't supply one.
        compute_asm_transition(spec, pre_state, &input.block.0, &input.aux_data, None)
            .expect("asm: compute transition")
    }

    fn extract_post_state(output: &Self::StepOutput) -> &Self::State {
        &output.state
    }

    fn extract_next_predicate(output: &Self::StepOutput) -> Option<PredicateKey> {
        // Iterate through each AsmLog; if we find an AsmStfUpdate, grab its vk and return it.
        output.manifest.logs.iter().find_map(|log| {
            log.try_into_log::<AsmStfUpdate>()
                .ok()
                .map(|update| update.new_predicate().clone())
        })
    }

    fn compute_export_state(export_state: ExportState, output: &Self::StepOutput) -> ExportState {
        // Iterate through each AsmLog; if we find an NewExportEntry, add it to ExportState
        let mut new_export_state = export_state;
        for log in &output.manifest.logs {
            if let Ok(export) = log.try_into_log::<NewExportEntry>() {
                new_export_state
                    .add_entry(export.container_id(), *export.entry_data())
                    .expect("failed to add entry");
            }
        }
        new_export_state
    }
}
