//! Block phases and admission checks that the sequencer composes itself.
//!
//! Sequencer block assembly stages each transaction separately so it can drop
//! failing ones, so it composes these phases in `strata-ol-block-assembly`
//! instead of calling [`construct_block`](crate::construct_block). Its mempool
//! uses [`check_tx_constraints`] to admit transactions. A rule set that changes
//! the order or composition of these phases must also update sequencer block
//! assembly, which no dispatch here enforces.

// TODO(STR-2863): deblockification removes block start and replaces these
// per-block phases with epoch-level transaction processing.

use strata_acct_types::{AccountId, TxEffects};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockBodyV1, OLBlockHeaderV1};
use strata_ol_state_types::{IAccountState, IStateAccessorMut, OLSpecId, TxProofIndexer};
use strata_ol_stf_v1::{
    BasicExecContext, BlockContext, ExecResult, ManifestProcessingOutcome, TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1, TxConstraintsV1};

use crate::spec::{OLStfSpec, dispatch_spec};
use crate::v1::StfV1;

/// Opens a block under `spec`: epoch-initial processing for the first block of
/// an epoch, then block-start processing.
///
/// See [`strata_ol_stf_v1::execute_block_initialization`].
pub fn execute_block_initialization<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    block_context: &BlockContext<'_>,
) -> ExecResult<()> {
    dispatch_spec!(spec => execute_block_initialization(state, block_context))
}

/// Processes one transaction under `spec`.
///
/// See [`strata_ol_stf_v1::process_single_tx`].
pub fn process_single_tx<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    tx: &OLTransactionV1,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    dispatch_spec!(spec => process_single_tx(state, tx, context))
}

/// Checks a transaction's constraints against `state` under `spec`.
///
/// See [`strata_ol_stf_v1::check_tx_constraints`].
// TODO(STR-2863): transaction constraints move from slots to epochs.
pub fn check_tx_constraints<S: IStateAccessorMut>(
    spec: OLSpecId,
    constraints: &TxConstraintsV1,
    state: &S,
) -> ExecResult<()> {
    dispatch_spec!(spec => check_tx_constraints(constraints, state))
}

/// Lists the accumulator proofs and predicate checks a snark account update
/// needs under `spec`, without verifying any of them.
///
/// Block assembly attaches the listed accumulator proofs before it executes
/// the update.
///
/// # Errors
///
/// Returns an error if the target is not a snark account or the update fails a
/// check that needs no proof, such as its sequence number.
pub fn index_snark_update_proof_requirements(
    spec: OLSpecId,
    target: AccountId,
    account_state: &impl IAccountState,
    sau_op: &SauTxOperationDataV1,
    effects: &TxEffects,
) -> ExecResult<TxProofIndexer> {
    dispatch_spec!(spec => index_snark_update_proof_requirements(
        target,
        account_state,
        sau_op,
        effects,
    ))
}

/// Buffers one ASM manifest's logs into intraepoch state under `spec`.
///
/// See [`strata_ol_stf_v1::process_asm_manifest`].
pub fn process_asm_manifest<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    manifest: &AsmManifest,
) -> ExecResult<ManifestProcessingOutcome> {
    dispatch_spec!(spec => process_asm_manifest(state, manifest))
}

/// Runs the epoch-terminal drain under `spec`.
///
/// The terminal block of an epoch runs under the spec of the epoch it ends,
/// even though the drain advances state to the next epoch. See
/// [`strata_ol_stf_v1::process_epoch_terminal`].
pub fn process_epoch_terminal<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    dispatch_spec!(spec => process_epoch_terminal(state, context))
}

/// Checks that a block's header and body are internally consistent under `spec`.
///
/// See [`strata_ol_stf_v1::verify_block_structure`].
pub fn verify_block_structure(
    spec: OLSpecId,
    header: &OLBlockHeaderV1,
    body: &OLBlockBodyV1,
) -> ExecResult<()> {
    dispatch_spec!(spec => verify_block_structure(header, body))
}
