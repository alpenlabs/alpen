//! The genesis OL rules, implemented by [`strata_ol_stf_v1`].

use strata_acct_types::{AccountId, TxEffects};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_da_types_v1::{OLDaSchemeV1, decode_ol_da_payload_bytes};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::{IAccountState, IStateAccessorMut, TxProofIndexer};
use strata_ol_stf_v1::{
    self as v1, BasicExecContext, BlockComponents, BlockContext, CompletedBlock,
    ConstructBlockOutput, EpochExecExpectations, EpochInfo, ExecResult, ManifestProcessingOutcome,
    TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1, TxConstraintsV1};

use crate::da::EpochDaReplayError;
use crate::spec::OLStfSpec;

/// The genesis OL rules.
#[derive(Debug)]
pub(crate) struct StfV1;

impl OLStfSpec for StfV1 {
    fn verify_block<S: IStateAccessorMut>(
        state: &mut S,
        header: &OLBlockHeaderV1,
        parent_header: Option<&OLBlockHeaderV1>,
        body: &OLBlockBodyV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        v1::verify_block(state, header, parent_header, body, runtime_params)
    }

    fn construct_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<ConstructBlockOutput> {
        v1::construct_block(state, block_context, block_components, runtime_params)
    }

    fn execute_and_complete_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<CompletedBlock> {
        v1::execute_and_complete_block(state, block_context, block_components, runtime_params)
    }

    fn execute_block_batch_predrain<S: IStateAccessorMut>(
        state: &mut S,
        blocks: &[OLBlockV1],
        initial_parent: &OLBlockHeaderV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        v1::execute_block_batch_predrain(state, blocks, initial_parent, runtime_params)
    }

    fn apply_da_epoch<S: IStateAccessorMut>(
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        let diff = decode_ol_da_payload_bytes(encoded_diff).map_err(EpochDaReplayError::Decode)?;
        v1::apply_da_epoch::<S, OLDaSchemeV1>(state, epoch_info, diff, manifests, runtime_params)
            .map_err(EpochDaReplayError::Exec)
    }

    fn verify_epoch_with_diff<S: IStateAccessorMut>(
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        exp: &EpochExecExpectations,
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        let diff = decode_ol_da_payload_bytes(encoded_diff).map_err(EpochDaReplayError::Decode)?;
        v1::verify_epoch_with_diff::<S, OLDaSchemeV1>(
            state,
            epoch_info,
            diff,
            manifests,
            exp,
            runtime_params,
        )
        .map_err(EpochDaReplayError::Exec)
    }

    fn execute_block_initialization<S: IStateAccessorMut>(
        state: &mut S,
        block_context: &BlockContext<'_>,
    ) -> ExecResult<()> {
        v1::execute_block_initialization(state, block_context)
    }

    fn process_single_tx<S: IStateAccessorMut>(
        state: &mut S,
        tx: &OLTransactionV1,
        context: &TxExecContext<'_>,
    ) -> ExecResult<()> {
        v1::process_single_tx(state, tx, context)
    }

    fn check_tx_constraints<S: IStateAccessorMut>(
        constraints: &TxConstraintsV1,
        state: &S,
    ) -> ExecResult<()> {
        v1::check_tx_constraints(constraints, state)
    }

    fn index_snark_update_proof_requirements(
        target: AccountId,
        account_state: &impl IAccountState,
        sau_op: &SauTxOperationDataV1,
        effects: &TxEffects,
    ) -> ExecResult<TxProofIndexer> {
        // The indexer records each proof check instead of verifying it, so the
        // update's proof checks run as a dry run that lists what it needs.
        let mut proof_indexer = TxProofIndexer::new_fresh();
        v1::verify_snark_acct_update_proofs(
            target,
            account_state,
            sau_op,
            effects,
            &mut proof_indexer,
        )?;
        Ok(proof_indexer)
    }

    fn process_asm_manifest<S: IStateAccessorMut>(
        state: &mut S,
        manifest: &AsmManifest,
    ) -> ExecResult<ManifestProcessingOutcome> {
        v1::process_asm_manifest(state, manifest)
    }

    fn process_epoch_terminal<S: IStateAccessorMut>(
        state: &mut S,
        context: &BasicExecContext<'_>,
    ) -> ExecResult<()> {
        v1::process_epoch_terminal(state, context)
    }

    fn verify_block_structure(header: &OLBlockHeaderV1, body: &OLBlockBodyV1) -> ExecResult<()> {
        v1::verify_block_structure(header, body)
    }
}
