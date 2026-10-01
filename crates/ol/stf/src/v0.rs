//! The V0 OL rules, which networks launched on 0.3.0 run from genesis.
//!
//! This binary implements V0 only for the work a node does on such a network
//! before its first V1 epoch: building the genesis block. Every other
//! operation returns [`ExecError::UnimplementedSpec`].
//!
//! [`StfV0`] reuses the [`strata_ol_stf_v1`] phase for every step whose 0.3.0
//! rules are the same, and implements here only the steps whose rules differ:
//!
//! - Epoch-initial processing does not wrap the state into the V1 root form, so
//!   a V0 state keeps the bare chainstate root that 0.3.0 headers commit to.

use strata_acct_types::{AccountId, TxEffects};
use strata_ol_chain_types_v1::{
    AsmManifest, Epoch, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog,
};
use strata_ol_params::{BridgeParams, OLRuntimeParams};
use strata_ol_state_types::{
    IAccountState, IStateAccessor, IStateAccessorMut, OLSpecId, TxProofIndexer,
};
use strata_ol_stf_v1::{
    self as v1, BasicExecContext, BlockComponents, BlockContext, BlockExecOutputs, CompletedBlock,
    ConstructBlockOutput, EpochExecExpectations, EpochInfo, ExecError, ExecOutputBuffer,
    ExecResult, ManifestProcessingOutcome, TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1, TxConstraintsV1};

use crate::da::EpochDaReplayError;
use crate::spec::OLStfSpec;

/// The V0 OL rules.
#[derive(Debug)]
pub(crate) struct StfV0;

impl OLStfSpec for StfV0 {
    fn verify_block<S: IStateAccessorMut>(
        _state: &mut S,
        _header: &OLBlockHeaderV1,
        _parent_header: Option<&OLBlockHeaderV1>,
        _body: &OLBlockBodyV1,
        _runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        Err(unimplemented())
    }

    fn construct_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<ConstructBlockOutput> {
        let exec_outputs =
            execute_genesis_block(state, &block_context, &block_components, runtime_params)?;
        Ok(v1::complete_block(
            &block_context,
            block_components,
            exec_outputs,
        ))
    }

    fn execute_and_complete_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<CompletedBlock> {
        Self::construct_block(state, block_context, block_components, runtime_params)
            .map(ConstructBlockOutput::into_completed_block)
    }

    fn execute_block_batch_predrain<S: IStateAccessorMut>(
        _state: &mut S,
        _blocks: &[OLBlockV1],
        _initial_parent: &OLBlockHeaderV1,
        _runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        Err(unimplemented())
    }

    fn apply_da_epoch<S: IStateAccessorMut>(
        _state: &mut S,
        _epoch_info: &EpochInfo,
        _encoded_diff: &[u8],
        _manifests: &[AsmManifest],
        _runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        Err(unimplemented().into())
    }

    fn verify_epoch_with_diff<S: IStateAccessorMut>(
        _state: &mut S,
        _epoch_info: &EpochInfo,
        _encoded_diff: &[u8],
        _manifests: &[AsmManifest],
        _exp: &EpochExecExpectations,
        _runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        Err(unimplemented().into())
    }

    fn execute_block_initialization<S: IStateAccessorMut>(
        _state: &mut S,
        _block_context: &BlockContext<'_>,
    ) -> ExecResult<()> {
        Err(unimplemented())
    }

    fn process_single_tx<S: IStateAccessorMut>(
        _state: &mut S,
        _tx: &OLTransactionV1,
        _context: &TxExecContext<'_>,
    ) -> ExecResult<()> {
        Err(unimplemented())
    }

    fn predict_tx_log_payloads(
        _tx: &OLTransactionV1,
        _bridge_params: &BridgeParams,
        _on_payload: impl FnMut(&[u8]),
    ) -> ExecResult<()> {
        Err(unimplemented())
    }

    fn check_tx_constraints<S: IStateAccessorMut>(
        _constraints: &TxConstraintsV1,
        _state: &S,
    ) -> ExecResult<()> {
        Err(unimplemented())
    }

    fn index_snark_update_proof_requirements(
        _target: AccountId,
        _account_state: &impl IAccountState,
        _sau_op: &SauTxOperationDataV1,
        _effects: &TxEffects,
    ) -> ExecResult<TxProofIndexer> {
        Err(unimplemented())
    }

    fn process_asm_manifest<S: IStateAccessorMut>(
        _state: &mut S,
        _manifest: &AsmManifest,
    ) -> ExecResult<ManifestProcessingOutcome> {
        Err(unimplemented())
    }

    fn process_epoch_terminal<S: IStateAccessorMut>(
        _state: &mut S,
        _context: &BasicExecContext<'_>,
    ) -> ExecResult<()> {
        Err(unimplemented())
    }

    fn verify_block_structure(_header: &OLBlockHeaderV1, _body: &OLBlockBodyV1) -> ExecResult<()> {
        Err(unimplemented())
    }
}

/// Returns the error for an operation this binary does not implement under V0.
fn unimplemented() -> ExecError {
    ExecError::UnimplementedSpec(OLSpecId::V0)
}

/// Executes the genesis block of a network launched on 0.3.0.
///
/// That block runs on a V0 state with no buffered ASM logs, has no parent,
/// carries no transactions or ASM manifests, and is the terminal block of
/// epoch 0. A state of a later spec returns [`ExecError::StateFromLaterSpec`].
/// Any other block is not V0 work this binary does, so it returns
/// [`ExecError::UnimplementedSpec`]. A genesis block on a state past epoch 0
/// fails the epoch check with [`ExecError::ContextEpochMismatch`] instead.
/// All of them reject before touching `state`.
fn execute_genesis_block<S: IStateAccessorMut>(
    state: &mut S,
    block_context: &BlockContext<'_>,
    block_components: &BlockComponents,
    runtime_params: &OLRuntimeParams,
) -> ExecResult<BlockExecOutputs> {
    require_v0_state(state)?;
    if !is_empty_genesis_block(state, block_context, block_components) {
        return Err(unimplemented());
    }

    process_epoch_initial(state, block_context.epoch())?;
    v1::process_block_start(state, block_context)?;

    // The genesis block buffers no ASM logs, so the terminal drain only resets
    // the intraepoch state and advances the epoch, as the 0.3.0 drain did.
    let output = ExecOutputBuffer::new_empty();
    let term_ctx = BasicExecContext::new(*block_context.block_info(), &output, runtime_params);
    v1::execute_epoch_terminal(state, &term_ctx)?;

    let state_root = state.compute_state_root()?;
    Ok(BlockExecOutputs::new(state_root, output.into_logs()))
}

/// Checks that `state` was produced under V0.
///
/// V0 rules never take over a state of a later spec, so all V0 work this
/// binary does starts from a V0 state.
fn require_v0_state(state: &impl IStateAccessor) -> ExecResult<()> {
    let state_spec_version = state.cur_spec_version();
    if state_spec_version != u32::from(OLSpecId::V0) {
        return Err(ExecError::StateFromLaterSpec {
            spec: OLSpecId::V0,
            state_spec_version,
        });
    }
    Ok(())
}

/// Returns whether a block on a V0 `state` is the genesis block of a network
/// launched on 0.3.0, as [`execute_genesis_block`] describes it.
fn is_empty_genesis_block(
    state: &impl IStateAccessor,
    block_context: &BlockContext<'_>,
    block_components: &BlockComponents,
) -> bool {
    state.pending_asm_logs_len() == 0
        && block_context.parent_header().is_none()
        && block_components.is_terminal()
        && block_components.tx_segment().txs().is_empty()
        && block_components
            .manifest_container()
            .is_none_or(|container| container.manifests().is_empty())
}

/// Runs the V0 epoch-initial processing for the start of `epoch`.
///
/// It checks that the state is in the epoch being started. V1's
/// [`process_epoch_initial`](strata_ol_stf_v1::process_epoch_initial) also
/// wraps a V0 state into the V1 root form; under V0 the state keeps its bare
/// chainstate root.
fn process_epoch_initial<S: IStateAccessor>(state: &S, epoch: Epoch) -> ExecResult<()> {
    let state_epoch = state.cur_epoch();
    if epoch != state_epoch {
        return Err(ExecError::ContextEpochMismatch(epoch, state_epoch));
    }
    Ok(())
}
