//! Computes the DA artifacts for a complete OL epoch.
//!
//! ASM manifests already come from L1, so epoch DA does not duplicate the
//! state effects derived from them. Full post-epoch state reconstruction
//! applies the epoch DA diff and then replays those manifests. The same
//! pre-drain execution that constructs the diff also produces the OL logs
//! carried alongside it by the checkpoint.

use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{
    DaAccumulatingState, DaAccumulationError, MemoryStateBaseLayer,
};
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf_v1::{ExecError, execute_block_batch_predrain};
use thiserror::Error;

/// Errors produced while computing epoch DA.
#[derive(Debug, Error)]
pub enum EpochDaError {
    /// The supplied epoch contains no blocks.
    #[error("cannot compute epoch DA without blocks")]
    NoBlocks,

    /// The final supplied block is not an epoch terminal.
    #[error("cannot compute epoch DA: final block is not terminal")]
    FinalBlockNotTerminal,

    /// A terminal block appears before the end of the supplied epoch.
    #[error("cannot compute epoch DA: terminal block appears before the end")]
    EarlyTerminal,

    /// Replaying the epoch blocks failed.
    #[error("epoch block replay failed: {0}")]
    BlockReplay(#[source] ExecError),

    /// Finalizing or encoding the accumulated DA failed.
    #[error("DA accumulation failed: {0}")]
    Accumulation(#[source] DaAccumulationError),

    /// The accumulator did not produce a DA blob.
    #[error("no DA blob produced after epoch replay")]
    MissingBlob,
}

/// Epoch DA bytes and OL logs computed from an epoch's blocks.
#[derive(Debug)]
pub struct EpochDaOutput {
    da_bytes: Vec<u8>,
    ol_logs: Vec<OLLog>,
}

impl EpochDaOutput {
    /// Consumes the output and returns the epoch DA bytes and OL logs.
    pub fn into_parts(self) -> (Vec<u8>, Vec<OLLog>) {
        (self.da_bytes, self.ol_logs)
    }
}

/// Computes the complete DA for an epoch and its OL logs.
///
/// ASM manifests are already available from L1, so epoch DA does not duplicate
/// the state effects reconstructed from them. The computation therefore
/// replays the epoch's blocks without applying the terminal ASM-log drain.
///
/// # Errors
///
/// Returns an error if the blocks do not form a complete epoch, block replay
/// fails, or the accumulated DA diff cannot be encoded.
pub fn compute_epoch_da(
    pre_epoch_state: OLStateV1,
    epoch_blocks: &[OLBlockV1],
    previous_terminal: &OLBlockHeaderV1,
    runtime_params: &OLRuntimeParams,
) -> Result<EpochDaOutput, EpochDaError> {
    let (terminal, preceding_blocks) = epoch_blocks.split_last().ok_or(EpochDaError::NoBlocks)?;
    if !terminal.header().is_terminal() {
        return Err(EpochDaError::FinalBlockNotTerminal);
    }
    if preceding_blocks
        .iter()
        .any(|block| block.header().is_terminal())
    {
        return Err(EpochDaError::EarlyTerminal);
    }

    let mut da_state = DaAccumulatingState::new(MemoryStateBaseLayer::new(pre_epoch_state));
    let ol_logs = execute_block_batch_predrain(
        &mut da_state,
        epoch_blocks,
        previous_terminal,
        runtime_params,
    )
    .map_err(EpochDaError::BlockReplay)?;

    let da_bytes = da_state
        .take_completed_epoch_da_blob()
        .map_err(EpochDaError::Accumulation)?
        .ok_or(EpochDaError::MissingBlob)?;

    Ok(EpochDaOutput { da_bytes, ol_logs })
}

#[cfg(test)]
mod tests {
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_stf_v1::BlockComponents;
    use strata_ol_stf_v1::test_utils::{
        epoch_runner_run_block as run_block, epoch_runner_run_genesis as run_genesis,
        make_genesis_state,
    };

    use super::{EpochDaError, compute_epoch_da};

    #[test]
    fn rejects_incomplete_epoch() {
        let mut state = make_genesis_state();
        let previous_terminal = run_genesis(&mut state);
        let pre_epoch_state = state.clone().into_inner();
        let runtime_params = OLRuntimeParams::test_default();

        let empty_error = compute_epoch_da(
            pre_epoch_state.clone(),
            &[],
            previous_terminal.header(),
            &runtime_params,
        )
        .expect_err("empty block list must not produce epoch DA");
        assert!(matches!(empty_error, EpochDaError::NoBlocks));

        let mut blocks = Vec::new();
        run_block(
            &mut state,
            &mut blocks,
            previous_terminal.header(),
            BlockComponents::new_empty(),
        );
        let nonterminal_error = compute_epoch_da(
            pre_epoch_state,
            &blocks,
            previous_terminal.header(),
            &runtime_params,
        )
        .expect_err("epoch DA must end at a terminal block");
        assert!(matches!(
            nonterminal_error,
            EpochDaError::FinalBlockNotTerminal
        ));
    }
}
