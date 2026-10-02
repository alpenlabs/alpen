//! The V0 OL rules, which networks launched on 0.3.0 run from genesis.
//!
//! This binary implements V0 only for the work a node does on such a network
//! before its first V1 epoch: building the genesis block, and replaying V0
//! epochs from checkpoint DA. A node on `main` never produces or verifies V0
//! blocks, so every other operation returns [`ExecError::UnimplementedSpec`].
//!
//! [`StfV0`] reuses the [`strata_ol_stf_v1`] code for every step whose V0
//! rules are the same, and implements here only the steps whose rules differ:
//!
//! - Epoch-initial processing does not wrap the state into the V1 root form, so a V0 state keeps
//!   the bare chainstate root that V0 headers commit to.
//! - Manifest buffering checks only that heights follow on, not where a checkpoint predicate
//!   enactment sits. The ASM accepts no checkpoint whose L1 range crosses one.
//! - The terminal drain applies an `EePredicateKeyUpdate` at once, setting the account's update
//!   key, where V1 queues a predicate-update inbox message. It hands deposits and checkpoint tip
//!   updates to the V1 handler and ignores every other log type, `CheckpointPredicateEnacted`
//!   included.
//!
//! The DA encoding is the V1 one, the V0 layout with `update_vk` appended
//! to the snark account diff, decoded by the V0 rules with
//! [`decode_v0_payload_bytes`]: presence bits past the V0 snark diff
//! members are ignored and `update_vk` stays unset.
//!
//! # Checks inherited from V1
//!
//! The V1 deposit handler and DA scheme V0 replay runs reject inputs V0 does
//! not check:
//!
//! - a deposit above the Bitcoin money supply;
//! - a DA balance or limbo change, or a new account's balance, beyond the money supply;
//! - a snark account key in the DA whose condition exceeds `MAX_CONDITION_LEN`.
//!
//! No provable V0 payload carries them. A V0 checkpoint proves that 0.3.0
//! applied its payload: bridge deposits are one denomination, so no balance
//! nears the supply, and 0.3.0 panicked converting a key past that length.

use strata_acct_types::{AccountId, L1BlockRecord, TxEffects};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::EePredicateKeyUpdate;
use strata_asm_logs::constants::AsmLogTypeId;
use strata_identifiers::L1Height;
use strata_ol_chain_types_v1::{
    AsmManifest, Epoch, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog,
};
use strata_ol_da_common::DaScheme;
use strata_ol_da_types_v1::{OLDaPayloadV1, OLDaSchemeV1, decode_v0_payload_bytes};
use strata_ol_params::{BridgeParams, OLRuntimeParams};
use strata_ol_state_types::{
    IAccountState, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor, IStateAccessorMut,
    OLSpecId, PendingAsmLog, TxProofIndexer,
};
use strata_ol_stf_v1::{
    self as v1, BasicExecContext, BlockComponents, BlockContext, BlockExecOutputs, CompletedBlock,
    ConstructBlockOutput, EpochExecExpectations, EpochInfo, ExecError, ExecOutputBuffer,
    ExecResult, ManifestProcessingOutcome, TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1, TxConstraintsV1};
use tracing::{debug, error, warn};

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
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        // Check the state before decoding, so a rejected replay never depends
        // on the diff.
        require_v0_state(state)?;
        let diff = decode_v0_payload_bytes(encoded_diff).map_err(EpochDaReplayError::Decode)?;
        replay_epoch(state, epoch_info, diff, manifests, runtime_params)
            .map_err(EpochDaReplayError::Exec)
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

    let output = ExecOutputBuffer::new_empty();
    let term_ctx = BasicExecContext::new(*block_context.block_info(), &output, runtime_params);
    process_epoch_terminal(state, &term_ctx)?;

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

/// Replays a V0 epoch from its decoded DA diff and its L1 manifests, as the
/// terminal block of that epoch would have left the state.
///
/// The diff carries the effects of the epoch's transactions; replaying the
/// manifests restores the ASM effects the diff leaves out. The caller checks
/// the resulting state root against the checkpoint.
fn replay_epoch<S: IStateAccessorMut>(
    state: &mut S,
    epoch_info: &EpochInfo,
    diff: OLDaPayloadV1,
    manifests: &[AsmManifest],
    runtime_params: &OLRuntimeParams,
) -> ExecResult<()> {
    process_epoch_initial(state, epoch_info.epoch())?;

    OLDaSchemeV1::apply_to_state(diff, state).map_err(|e| {
        error!(
            error = %e,
            epoch = epoch_info.epoch(),
            "DA scheme failed to apply diff during V0 epoch reconstruction"
        );
        ExecError::ChainIntegrity
    })?;

    process_manifests(state, manifests)?;
    let output = ExecOutputBuffer::new_empty();
    let term_ctx = BasicExecContext::new(epoch_info.terminal_info(), &output, runtime_params);
    process_epoch_terminal(state, &term_ctx)?;
    output.verify_logs_within_block_limit()?;
    Ok(())
}

/// Buffers the ASM logs of `manifests` into the intraepoch state and accepts
/// each manifest's L1 block record, as 0.3.0's `process_block_manifests` did.
///
/// Each manifest must be at the height after the state's last L1 height.
/// Unlike V1's buffering, this does not check where a checkpoint predicate
/// enactment sits among the manifests.
fn process_manifests<S: IStateAccessorMut>(
    state: &mut S,
    manifests: &[AsmManifest],
) -> ExecResult<()> {
    for (index, manifest) in manifests.iter().enumerate() {
        let height = state
            .last_l1_height()
            .checked_add(1)
            .ok_or(ExecError::AsmManifestHeightOverflow)?;
        if manifest.height() != height {
            return Err(ExecError::AsmManifestHeightMismatch {
                expected: height,
                actual: manifest.height(),
                index,
            });
        }

        for log in manifest.logs() {
            state.try_append_pending_asm_log(PendingAsmLog::new(height, log.clone()))?;
        }
        let record =
            L1BlockRecord::new(*manifest.blkid().as_ref(), *manifest.wtxids_root().as_ref());
        state.append_l1_block_rec(height, record);
    }
    Ok(())
}

/// Runs the V0 epoch-terminal drain: applies each buffered ASM log in
/// order, resets the intraepoch state, and advances the epoch.
fn process_epoch_terminal<S: IStateAccessorMut>(
    state: &mut S,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let terminating_epoch = state.cur_epoch();

    // The log handlers buffer nothing, so the snapshot stays complete.
    let pending: Vec<PendingAsmLog> = (0..state.pending_asm_logs_len())
        .map(|idx| {
            state
                .get_pending_asm_log(idx)
                .expect("pending asm log index within bounds")
        })
        .collect();
    for entry in &pending {
        process_asm_log(state, entry.log(), entry.height(), context)?;
    }

    state.reset_intraepoch_state();
    let new_epoch = terminating_epoch
        .checked_add(1)
        .ok_or(ExecError::EpochOverflow)?;
    state.set_cur_epoch(new_epoch);
    Ok(())
}

/// Applies the effects of one buffered ASM log under the V0 rules.
fn process_asm_log<S: IStateAccessorMut>(
    state: &mut S,
    log: &AsmLogEntry,
    height: L1Height,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let Some(ty) = log.ty() else {
        debug!(height, "skipping asm log: not an sps-52 message");
        return Ok(());
    };

    match AsmLogTypeId::try_from(ty) {
        Ok(AsmLogTypeId::Deposit | AsmLogTypeId::CheckpointTipUpdate) => {
            v1::process_asm_log(state, log, height, context)
        }
        Ok(AsmLogTypeId::EePredicateKeyUpdate) => {
            let Ok(update) = log.try_into_log::<EePredicateKeyUpdate>() else {
                debug!(
                    height,
                    "failed to decode ee predicate key update log; skipping"
                );
                return Ok(());
            };
            process_ee_predicate_key_update(state, &update)
        }
        // V0 acts on no other log type. `CheckpointPredicateEnacted` ends
        // the last V0 epoch; the epoch after it runs under V1.
        _ => {
            debug!(height, ty, "ignoring asm log type under V0 rules");
            Ok(())
        }
    }
}

/// Applies an `EePredicateKeyUpdate` the V0 way: the target snark account's
/// update key becomes the new predicate at once.
///
/// V0 does not check the predicate's type id, and it skips an unknown
/// account serial or a non-snark account.
fn process_ee_predicate_key_update<S: IStateAccessorMut>(
    state: &mut S,
    update: &EePredicateKeyUpdate,
) -> ExecResult<()> {
    let acct_serial = update.account();
    let Some(acct_id) = state.find_account_id_by_serial(acct_serial)? else {
        warn!(
            ?acct_serial,
            "dropping ee predicate key update for unknown account serial"
        );
        return Ok(());
    };

    let new_vk = update.new_predicate().clone();
    let applied =
        state.update_account(acct_id, |account| match account.as_snark_account_mut() {
            Ok(snark) => {
                snark.set_update_vk(new_vk);
                true
            }
            Err(_) => false,
        })?;
    if !applied {
        warn!(
            %acct_serial,
            %acct_id,
            "dropping ee predicate key update for non-snark account"
        );
    }
    Ok(())
}
