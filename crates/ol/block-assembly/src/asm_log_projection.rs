//! ASM-log projection helpers for OL block assembly.

use strata_bridge_params::BridgeParams;
use strata_ledger_types::{IAccountStateMut, IStateAccessor, IStateAccessorMut, PendingAsmLog};
use strata_ol_chain_types_new::{AsmManifest, MAX_LOGS_PER_BLOCK, OLLog};
use strata_ol_state_support_types::DaAccumulatingState;
use strata_ol_stf::{
    BasicExecContext, BlockContext, ExecError, ExecOutputBuffer, process_pending_asm_log_effect,
};

use crate::{
    BlockAssemblyError, BlockAssemblyResult, checkpoint_size::LogMetrics,
    epoch_sealing::EpochSealingResourceStats,
};

/// Scratch projection of buffered ASM-log effects if the candidate block is terminal.
///
/// The projection starts from the candidate post-transaction state and applies
/// pending ASM-log effects into scratch DA/output trackers. It lets block
/// assembly ask sealing-limit rules about the resource usage that would
/// materialize if the candidate block sealed the epoch.
pub(crate) struct ProjectedAsmLogOutput<S: IStateAccessorMut> {
    state: DaAccumulatingState<S>,
    output_buffer: ExecOutputBuffer,
    epoch_log_metrics: LogMetrics,
}

impl<S> ProjectedAsmLogOutput<S>
where
    S: IStateAccessorMut,
    S::AccountStateMut: Clone,
    <S::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    /// Creates a buffered ASM-log projection rooted at the candidate post-tx state.
    ///
    /// Seeds the projected block output with transaction logs already selected
    /// for the candidate block, then projects all ASM logs already pending in
    /// the parent state. Returns an error when already-pending logs alone would
    /// exceed the terminal block's log limit, because there is no newly fetched
    /// manifest candidate to reject in that case.
    pub(crate) fn new(
        state: DaAccumulatingState<S>,
        epoch_log_metrics: LogMetrics,
        block_logs_after_txs: &[OLLog],
        block_context: &BlockContext<'_>,
        bridge_params: BridgeParams,
    ) -> BlockAssemblyResult<Self> {
        let output_buffer = ExecOutputBuffer::new_empty();
        output_buffer
            .emit_logs(block_logs_after_txs.iter().cloned())
            .map_err(BlockAssemblyError::BlockConstruction)?;

        let mut projection = Self {
            state,
            output_buffer,
            epoch_log_metrics,
        };
        projection.project_existing_pending_logs(block_context, bridge_params)?;
        Ok(projection)
    }

    fn project_existing_pending_logs(
        &mut self,
        block_context: &BlockContext<'_>,
        bridge_params: BridgeParams,
    ) -> BlockAssemblyResult<()> {
        let pending_count = self.state.pending_asm_logs_len();
        for idx in 0..pending_count {
            let entry = self
                .state
                .get_pending_asm_log(idx)
                .expect("pending ASM log index within bounds");
            if !self.project_pending_log_effect(&entry, block_context, bridge_params)? {
                return Err(BlockAssemblyError::Other(
                    "projected buffered ASM-log processing exceeds the block log limit before admitting new manifests"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Projects buffered ASM-log effects from one candidate ASM manifest.
    ///
    /// Returns `Ok(false)` when applying the manifest would exceed the
    /// terminal block's log limit. Other STF failures are returned as block
    /// assembly errors.
    pub(crate) fn project_manifest(
        &mut self,
        manifest: &AsmManifest,
        block_context: &BlockContext<'_>,
        bridge_params: BridgeParams,
    ) -> BlockAssemblyResult<bool> {
        for log in manifest.logs() {
            let entry = PendingAsmLog::new(manifest.height(), log.clone());
            if !self.project_pending_log_effect(&entry, block_context, bridge_params)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn project_pending_log_effect(
        &mut self,
        entry: &PendingAsmLog,
        block_context: &BlockContext<'_>,
        bridge_params: BridgeParams,
    ) -> BlockAssemblyResult<bool> {
        let old_log_count = self.output_buffer.log_count();
        let basic_ctx = BasicExecContext::new(*block_context.block_info(), &self.output_buffer)
            .with_bridge_params(bridge_params);

        match process_pending_asm_log_effect(&mut self.state, entry, &basic_ctx) {
            Ok(()) => {
                self.record_projected_logs(old_log_count);
                Ok(true)
            }
            Err(ExecError::LogsOverflow { .. }) => Ok(false),
            Err(err) => Err(BlockAssemblyError::BlockConstruction(err)),
        }
    }

    fn record_projected_logs(&mut self, old_log_count: usize) {
        let logs = self.output_buffer.snapshot_logs();
        if logs.len() > old_log_count {
            self.epoch_log_metrics.add_logs(&logs[old_log_count..]);
        }
    }

    /// Returns sealing resource stats for the current projected manifest prefix.
    ///
    /// The stats include epoch-cumulative DA/log metrics and the projected log
    /// count for this block if it is terminal.
    pub(crate) fn stats(&self, manifest_count: usize) -> EpochSealingResourceStats {
        EpochSealingResourceStats::new(
            self.state.accumulator().estimated_encoded_size(),
            self.epoch_log_metrics,
        )
        .with_manifest_count(manifest_count)
        .with_terminal_block_log_count(self.output_buffer.log_count())
    }

    /// Returns stats that force the terminal block log-count rule to reject.
    pub(crate) fn stats_with_block_log_overflow(
        &self,
        manifest_count: usize,
    ) -> EpochSealingResourceStats {
        self.stats(manifest_count)
            .with_terminal_block_log_count((MAX_LOGS_PER_BLOCK as usize).saturating_add(1))
    }
}
