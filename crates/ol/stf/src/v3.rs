//! The V3 OL rules, which follow V2.
//!
//! [`StfV3`] runs every operation with the [`strata_ol_stf_v1`] code, as V2
//! does, and takes over a V2 state at the first block of an epoch, as V2
//! takes over a V1 state. [`CappedV1Rules`] implements the operations V3
//! shares with V2, and rules that V3 changes later replace the call of the
//! operation they change.
//!
//! # Rules V3 changes
//!
//! The checkpoint programs that prove V1 and V2 epochs are published, so
//! those rules cannot change: a node must accept exactly what the programs
//! prove. V3 carries the fixes made after the V2 program was built:
//!
//! - A message payload holds at most the 4 KiB the epoch DA encodes for an inbox message
//!   ([`OLSpecLimits`]). Earlier specs accept any payload the SSZ types allow.
//! - A withdrawal message to the bridge gateway writes its descriptor length in the shortest
//!   layout ([`RuleAmendments::V3`]). Earlier specs take every layout.
//! - An accumulator proof for an inbox message or an L1 block ref leads to the peak that holds
//!   the claimed index ([`RuleAmendments::V3`]). Earlier specs select the peak by the proof's
//!   length and check the index only within it.
//! - An epoch replays only from the DA bytes the encoder writes ([`DaDiffEncoding::Canonical`]).
//!   Earlier specs replay every encoding the decoders read.
//!
//! # Taking over a V2 state
//!
//! The epoch after a V2 epoch that processed a checkpoint predicate enactment
//! runs V3. V2 rules never stage a spec, so that epoch starts from a V2
//! terminal state with `cur = staged = V2`. Its first block records V3 as
//! `cur = staged = V3`, which the state root commits from that block on, and
//! which node-side spec selection reads as the epoch's spec. V3 rules never
//! stage a spec either.
//!
//! V3 takes over nothing else. The operations that open a block or replay an
//! epoch fail, without touching the state, on a V0 or V1 state, on a V2 state
//! outside an epoch start, and on a state of a later spec. [`CappedV1Rules`]
//! describes how each operation takes over the state.

use strata_acct_types::{AccountId, TxEffects};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_params::{BridgeParams, OLRuntimeParams};
use strata_ol_state_types::{IAccountState, IStateAccessorMut, OLSpecId, TxProofIndexer};
use strata_ol_stf_v1::{
    self as v1, BasicExecContext, BlockComponents, BlockContext, CompletedBlock,
    ConstructBlockOutput, EpochExecExpectations, EpochInfo, ExecResult, ManifestProcessingOutcome,
    RuleAmendments, TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1, TxConstraintsV1};

use crate::capped_v1::{CappedV1Rules, DaDiffEncoding};
use crate::da::EpochDaReplayError;
use crate::limits::OLSpecLimits;
use crate::spec::OLStfSpec;

/// The V3 OL rules.
#[derive(Debug)]
pub(crate) struct StfV3;

impl StfV3 {
    /// V3 runs the V1 code with the V3 amendments, within the V3 capacities,
    /// takes over a V2 state, and replays only the canonical DA encoding.
    const RULES: CappedV1Rules = CappedV1Rules::new(
        OLSpecId::V3,
        OLSpecId::V2,
        OLSpecLimits::V3,
        RuleAmendments::V3,
        DaDiffEncoding::Canonical,
    );
}

impl OLStfSpec for StfV3 {
    fn limits() -> OLSpecLimits {
        Self::RULES.limits()
    }

    fn verify_block<S: IStateAccessorMut>(
        state: &mut S,
        header: &OLBlockHeaderV1,
        parent_header: Option<&OLBlockHeaderV1>,
        body: &OLBlockBodyV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        Self::RULES.verify_block(state, header, parent_header, body, runtime_params)
    }

    fn construct_block<S: IStateAccessorMut>(
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<ConstructBlockOutput> {
        Self::RULES.construct_block(state, block_context, block_components, runtime_params)
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
        Self::RULES.execute_block_batch_predrain(state, blocks, initial_parent, runtime_params)
    }

    fn apply_da_epoch<S: IStateAccessorMut>(
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        Self::RULES.apply_da_epoch(state, epoch_info, encoded_diff, manifests, runtime_params)
    }

    fn verify_epoch_with_diff<S: IStateAccessorMut>(
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        exp: &EpochExecExpectations,
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        Self::RULES.verify_epoch_with_diff(
            state,
            epoch_info,
            encoded_diff,
            manifests,
            exp,
            runtime_params,
        )
    }

    fn execute_block_initialization<S: IStateAccessorMut>(
        state: &mut S,
        block_context: &BlockContext<'_>,
    ) -> ExecResult<()> {
        Self::RULES.execute_block_initialization(state, block_context)
    }

    fn process_single_tx<S: IStateAccessorMut>(
        state: &mut S,
        tx: &OLTransactionV1,
        context: &TxExecContext<'_>,
    ) -> ExecResult<()> {
        Self::RULES.process_single_tx(state, tx, context)
    }

    fn predict_tx_log_payloads(
        tx: &OLTransactionV1,
        bridge_params: &BridgeParams,
        on_payload: impl FnMut(&[u8]),
    ) -> ExecResult<()> {
        Self::RULES.predict_tx_log_payloads(tx, bridge_params, on_payload)
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
        Self::RULES.index_snark_update_proof_requirements(target, account_state, sau_op, effects)
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
        Self::RULES.process_epoch_terminal(state, context)
    }

    fn verify_block_structure(header: &OLBlockHeaderV1, body: &OLBlockBodyV1) -> ExecResult<()> {
        v1::verify_block_structure(header, body)
    }
}
