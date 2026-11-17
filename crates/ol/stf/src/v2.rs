//! The V2 OL rules, which follow V1.
//!
//! V2 starts as a copy of the V1 rules: [`StfV2`] runs every operation with
//! the [`strata_ol_stf_v1`] code, except that it takes over a V1 state at the
//! first block of an epoch, as V1 takes over a V0 state, and that it allows
//! fewer transaction effects and block logs and smaller message payloads.
//! [`CappedV1Rules`] implements the operations V2 shares with V3, and rules
//! that V2 changes later replace the call of the operation they change.
//!
//! # Capacities
//!
//! V2 restores the capacities of the 0.3.0 rules, which V1 raised: a
//! transaction carries at most 255 transfers and 255 messages, and a block
//! emits at most 4,096 logs ([`OLSpecLimits`]). V2 also bounds each message
//! payload at the 4 KiB the epoch DA encodes for an inbox message, which no
//! earlier spec checks. The V1 types keep their larger bounds, so V2 checks
//! its capacities around the V1 code instead, as [`CappedV1Rules`] describes.
//! A transaction or block above a capacity is invalid.
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
//! V2 takes over nothing else. The operations that open a block or replay an
//! epoch fail, without touching the state, on a V0 state, on a V1 state
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

use crate::capped_v1::CappedV1Rules;
use crate::da::EpochDaReplayError;
use crate::limits::OLSpecLimits;
use crate::spec::OLStfSpec;

/// The V2 OL rules.
#[derive(Debug)]
pub(crate) struct StfV2;

impl StfV2 {
    /// V2 runs the V1 code without amendments, within the V2 capacities, and
    /// takes over a V1 state.
    const RULES: CappedV1Rules = CappedV1Rules::new(
        OLSpecId::V2,
        OLSpecId::V1,
        OLSpecLimits::V2,
        RuleAmendments::NONE,
    );
}

impl OLStfSpec for StfV2 {
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
