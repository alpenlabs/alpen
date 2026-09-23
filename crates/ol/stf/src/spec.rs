//! The operations an OL rules version implements, and dispatch by spec.

use strata_acct_types::{AccountId, TxEffects};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::{IAccountState, IStateAccessorMut, TxProofIndexer};
use strata_ol_stf_v1::{
    BasicExecContext, BlockComponents, BlockContext, CompletedBlock, ConstructBlockOutput,
    EpochExecExpectations, EpochInfo, ExecResult, ManifestProcessingOutcome, TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1, TxConstraintsV1};

use crate::da::EpochDaReplayError;

/// The operations an OL rules version implements.
///
/// Each implementation is one complete rule set: block execution, transaction
/// and proof processing, manifest processing, and the DA encoding its epochs
/// use. [`dispatch_spec!`] selects the implementation for an
/// [`OLSpecId`](strata_ol_state_types::OLSpecId). The public functions of this
/// crate document each operation.
///
/// The signatures use the V1 chain, context, and error types. A rule set that
/// changes those types needs associated types here.
// TODO(STR-2863): deblockification reduces the state-changing operations to
// epoch-initial, epoch-transaction, and epoch check-in processing. Shrink this
// trait to match.
pub(crate) trait OLStfSpec {
    fn verify_block<S: IStateAccessorMut>(
        state: &mut S,
        header: &OLBlockHeaderV1,
        parent_header: Option<&OLBlockHeaderV1>,
        body: &OLBlockBodyV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>>;

    fn construct_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<ConstructBlockOutput>;

    fn execute_and_complete_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<CompletedBlock>;

    fn execute_block_batch_predrain<S: IStateAccessorMut>(
        state: &mut S,
        blocks: &[OLBlockV1],
        initial_parent: &OLBlockHeaderV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>>;

    fn apply_da_epoch<S: IStateAccessorMut>(
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError>;

    fn verify_epoch_with_diff<S: IStateAccessorMut>(
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        exp: &EpochExecExpectations,
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError>;

    fn execute_block_initialization<S: IStateAccessorMut>(
        state: &mut S,
        block_context: &BlockContext<'_>,
    ) -> ExecResult<()>;

    fn process_single_tx<S: IStateAccessorMut>(
        state: &mut S,
        tx: &OLTransactionV1,
        context: &TxExecContext<'_>,
    ) -> ExecResult<()>;

    fn check_tx_constraints<S: IStateAccessorMut>(
        constraints: &TxConstraintsV1,
        state: &S,
    ) -> ExecResult<()>;

    fn index_snark_update_proof_requirements(
        target: AccountId,
        account_state: &impl IAccountState,
        sau_op: &SauTxOperationDataV1,
        effects: &TxEffects,
    ) -> ExecResult<TxProofIndexer>;

    fn process_asm_manifest<S: IStateAccessorMut>(
        state: &mut S,
        manifest: &AsmManifest,
    ) -> ExecResult<ManifestProcessingOutcome>;

    fn process_epoch_terminal<S: IStateAccessorMut>(
        state: &mut S,
        context: &BasicExecContext<'_>,
    ) -> ExecResult<()>;

    fn verify_block_structure(header: &OLBlockHeaderV1, body: &OLBlockBodyV1) -> ExecResult<()>;
}

/// Calls operation `$op` on the [`OLStfSpec`] implementation that executes
/// `$spec`.
///
/// This is the only mapping from spec identifiers to implementations. Every
/// predicate rotation advances the spec, including rotations that change no
/// rules, so several specs can map to one implementation. A spec that reuses
/// earlier rules adds an arm naming an existing implementation; a spec with new
/// rules also adds an [`OLStfSpec`] implementation.
///
/// Callers import `OLSpecId`, `OLStfSpec`, and the implementation types.
macro_rules! dispatch_spec {
    ($spec:expr => $op:ident($($arg:expr),* $(,)?)) => {
        match $spec {
            OLSpecId::V1 => <StfV1 as OLStfSpec>::$op($($arg),*),
        }
    };
}

pub(crate) use dispatch_spec;
