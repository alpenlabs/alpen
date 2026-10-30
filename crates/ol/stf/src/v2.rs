//! The V2 OL rules, which follow V1.
//!
//! V2 starts as a copy of the V1 rules: [`StfV2`] runs every operation with
//! the [`strata_ol_stf_v1`] code, except that it takes over a V1 state at the
//! first block of an epoch, as V1 takes over a V0 state, and that it allows
//! fewer transaction effects and block logs. Rules that V2 changes later
//! replace the V1 call of the operation they change.
//!
//! # Capacities
//!
//! V2 restores the capacities of the 0.3.0 rules, which V1 raised: a
//! transaction carries at most 255 transfers and 255 messages, and a block
//! emits at most 4,096 logs ([`OLSpecLimits`]). The V1 types keep their
//! larger bounds, so V1 txids and encodings never change, and V2 checks its
//! capacities around the V1 code instead. Each operation that executes a
//! transaction checks its effects before the V1 code runs it, and each
//! operation that executes a whole block checks the block's logs after the
//! V1 code emits them. A transaction or block above a capacity is invalid.
//! Block assembly composes the [`sequencer`](crate::sequencer) phases itself,
//! so it checks the block log capacity on its own.
//!
//! # Taking over a V1 state
//!
//! The epoch after a V1 epoch that processed a checkpoint predicate enactment
//! runs V2. V1 rules never stage a spec, so that epoch starts from a V1
//! terminal state with `cur = staged = V1`. Its first block records V2 as
//! `cur = staged = V2`, which the state root commits from that block on, and
//! which node-side spec selection reads as the epoch's spec. V2 rules never
//! stage a spec either.
//!
//! Each operation that opens a block or replays an epoch takes over the state
//! before it runs the V1 code, so every driver, DA replay included, reaches
//! the same root. The V1 code then leaves the versions as they are: its
//! epoch-initial processing wraps only a V0 state, and it rejects only a V0
//! state outside an epoch start. Taking over before the V1 code checks the
//! block changes nothing on success, and callers discard the state of a failed
//! operation.
//!
//! V2 takes over nothing else. The operations that open a block or replay an
//! epoch fail, without touching the state, on a V0 state, and on a V1 state
//! outside an epoch start: each enactment advances the spec by one, so only a
//! wrong spec selection reaches either. The other operations run the V1 code
//! without checking the versions. The transaction, manifest, and
//! epoch-terminal phases run inside a block that
//! [`OLStfSpec::execute_block_initialization`] opened, and mempool admission
//! checks a transaction for the first V2 block against the V1 state that
//! block takes over.

use std::slice;

use strata_acct_types::{AccountId, TxEffects};
use strata_ol_chain_types_v1::{
    AsmManifest, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog, OLTxSegmentV1,
};
use strata_ol_da_types_v1::{OLDaSchemeV1, decode_ol_da_payload_bytes};
use strata_ol_params::{BridgeParams, OLRuntimeParams};
use strata_ol_state_types::{
    IAccountState, IStateAccessorMut, OLSpecId, OLSpecVersions, TxProofIndexer,
};
use strata_ol_stf_v1::{
    self as v1, BasicExecContext, BlockComponents, BlockContext, CompletedBlock,
    ConstructBlockOutput, EpochExecExpectations, EpochInfo, ExecError, ExecResult,
    ManifestProcessingOutcome, TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1, TxConstraintsV1};

use crate::da::EpochDaReplayError;
use crate::limits::OLSpecLimits;
use crate::spec::OLStfSpec;

/// The V2 OL rules.
#[derive(Debug)]
pub(crate) struct StfV2;

impl OLStfSpec for StfV2 {
    fn limits() -> OLSpecLimits {
        OLSpecLimits::V2
    }

    fn verify_block<S: IStateAccessorMut>(
        state: &mut S,
        header: &OLBlockHeaderV1,
        parent_header: Option<&OLBlockHeaderV1>,
        body: &OLBlockBodyV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        take_over_state(state, opens_epoch(parent_header))?;
        if let Some(tx_segment) = body.tx_segment() {
            check_tx_segment_effects(tx_segment)?;
        }
        let logs = v1::verify_block(state, header, parent_header, body, runtime_params)?;
        Self::limits().check_block_log_count(logs.len())?;
        Ok(logs)
    }

    fn construct_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<ConstructBlockOutput> {
        take_over_state(state, block_context.is_epoch_initial())?;
        check_tx_segment_effects(block_components.tx_segment())?;
        let output = v1::construct_block(state, block_context, block_components, runtime_params)?;
        Self::limits().check_block_log_count(output.outputs().logs().len())?;
        Ok(output)
    }

    fn execute_and_complete_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<CompletedBlock> {
        // The V1 operation drops the logs this needs to count, so this
        // completes the block the way the V1 operation does.
        Self::construct_block(state, block_context, block_components, runtime_params)
            .map(ConstructBlockOutput::into_completed_block)
    }

    fn execute_block_batch_predrain<S: IStateAccessorMut>(
        state: &mut S,
        blocks: &[OLBlockV1],
        initial_parent: &OLBlockHeaderV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        // Without the epoch-terminal drain the state stays in the epoch of the
        // first block, so only that block can open an epoch.
        if !blocks.is_empty() {
            take_over_state(state, opens_epoch(Some(initial_parent)))?;
        }
        // The V1 operation concatenates the logs of all blocks, so this runs
        // it one block at a time to count each block's logs.
        let mut batch_logs = Vec::new();
        let mut parent = initial_parent;
        for block in blocks {
            if let Some(tx_segment) = block.body().tx_segment() {
                check_tx_segment_effects(tx_segment)?;
            }
            let logs = v1::execute_block_batch_predrain(
                state,
                slice::from_ref(block),
                parent,
                runtime_params,
            )?;
            Self::limits().check_block_log_count(logs.len())?;
            batch_logs.extend(logs);
            parent = block.header();
        }
        Ok(batch_logs)
    }

    fn apply_da_epoch<S: IStateAccessorMut>(
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        // An epoch replay always opens the epoch. Taking over first checks the
        // state before decoding, so a rejected replay never depends on the
        // diff.
        take_over_state(state, true)?;
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
        take_over_state(state, true)?;
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
        take_over_state(state, block_context.is_epoch_initial())?;
        v1::execute_block_initialization(state, block_context)
    }

    fn process_single_tx<S: IStateAccessorMut>(
        state: &mut S,
        tx: &OLTransactionV1,
        context: &TxExecContext<'_>,
    ) -> ExecResult<()> {
        Self::limits().check_tx_effects(tx.data().effects())?;
        v1::process_single_tx(state, tx, context)
    }

    fn predict_tx_log_payloads(
        tx: &OLTransactionV1,
        bridge_params: &BridgeParams,
        on_payload: impl FnMut(&[u8]),
    ) -> ExecResult<()> {
        v1::predict_tx_log_payloads(tx, bridge_params, on_payload).map_err(Into::into)
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
        Self::limits().check_tx_effects(effects)?;
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

/// Checks the effects of each transaction in a block against the V2
/// capacities.
///
/// # Errors
///
/// Returns [`ExecError::TxEffectsOverflow`] for the first transaction above a
/// capacity, wrapped with its txid and index as block execution wraps
/// transaction errors.
fn check_tx_segment_effects(tx_segment: &OLTxSegmentV1) -> ExecResult<()> {
    let limits = StfV2::limits();
    for (index, tx) in tx_segment.txs().iter().enumerate() {
        limits
            .check_tx_effects(tx.data().effects())
            .map_err(|err| ExecError::from(err).with_tx(tx.compute_txid(), index))?;
    }
    Ok(())
}

/// Returns whether the block after `parent_header` is the first block of an
/// epoch, as [`BlockContext::is_epoch_initial`] decides it.
fn opens_epoch(parent_header: Option<&OLBlockHeaderV1>) -> bool {
    parent_header.is_none_or(OLBlockHeaderV1::is_terminal)
}

/// Takes over a V1 state at the first block of an epoch, and checks that any
/// other block runs on a V2 state.
///
/// `opens_epoch` says whether the work starts an epoch. A V2 state is left as
/// it is.
///
/// # Errors
///
/// Returns [`ExecError::ContinuesEarlierSpecEpoch`] for a V1 state outside an
/// epoch start, and [`ExecError::StateFromEarlierSpec`] for a V0 state. Neither
/// touches the state.
fn take_over_state<S: IStateAccessorMut>(state: &mut S, opens_epoch: bool) -> ExecResult<()> {
    match state.spec_versions().cur_spec() {
        OLSpecId::V2 => Ok(()),
        OLSpecId::V1 if opens_epoch => {
            state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V2));
            Ok(())
        }
        OLSpecId::V1 => Err(ExecError::ContinuesEarlierSpecEpoch {
            spec: OLSpecId::V2,
            state_spec: OLSpecId::V1,
        }),
        OLSpecId::V0 => Err(ExecError::StateFromEarlierSpec {
            spec: OLSpecId::V2,
            state_spec: OLSpecId::V0,
        }),
    }
}
