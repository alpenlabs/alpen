//! Computes the DA artifacts for a complete OL epoch.
//!
//! ASM manifests already come from L1, so epoch DA does not duplicate the
//! state effects derived from them. Full post-epoch state reconstruction
//! applies the epoch DA diff and then replays those manifests. The same
//! pre-drain execution that constructs the diff also produces the OL logs
//! carried alongside it by the checkpoint.
//!
//! The replay runs under the epoch's spec, but the accumulated diff is still
//! encoded as a V1 DA payload. Rules that change the DA format must also
//! select the encoder here.

use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{DaAccumulatingState, DaAccumulationError};
use strata_ol_state_types::IStateAccessorMut;
use strata_ol_stf::{ExecError, OLSpecId, execute_block_batch_predrain};
use thiserror::Error;

/// Errors produced while computing epoch DA.
#[derive(Debug, Error)]
pub enum EpochDaError {
    /// The supplied epoch contains no blocks.
    #[error("cannot compute epoch DA without blocks")]
    NoBlocks,

    /// The final supplied block is not an epoch terminal.
    #[error("cannot compute epoch DA; final block is not terminal")]
    FinalBlockNotTerminal,

    /// A terminal block appears before the end of the supplied epoch.
    #[error("cannot compute epoch DA; terminal block appears before the end")]
    EarlyTerminal,

    /// Replaying the epoch blocks failed.
    #[error("epoch block replay failed: {0}")]
    BlockReplay(#[source] ExecError),

    /// Finalizing or encoding the accumulated DA failed.
    #[error("DA accumulation failed: {0}")]
    Accumulation(#[source] DaAccumulationError),
}

/// Version-agnostic artifacts computed by replaying an epoch's blocks.
///
/// The DA encoding is intentionally opaque. Consumers interpret `encoded_da`
/// using the STF and DA scheme selected for the epoch's protocol version.
#[derive(Debug)]
pub struct EpochReplayArtifacts<Log> {
    encoded_da: Vec<u8>,
    ol_logs: Vec<Log>,
}

impl<Log> EpochReplayArtifacts<Log> {
    /// Consumes the output and returns the epoch DA bytes and OL logs.
    pub fn into_parts(self) -> (Vec<u8>, Vec<Log>) {
        (self.encoded_da, self.ol_logs)
    }
}

/// Computes the complete DA for an epoch and its OL logs, replaying the
/// epoch's blocks under `spec`.
///
/// ASM manifests are already available from L1, so epoch DA does not duplicate
/// the state effects reconstructed from them. The computation therefore
/// replays the epoch's blocks without applying the terminal ASM-log drain.
/// Log collection begins after epoch-initial processing. This is currently
/// complete because epoch-initial processing emits no logs; include that
/// phase's output here if this changes.
///
/// # Errors
///
/// Returns an error if the blocks do not form a complete epoch, block replay
/// fails, or the accumulated DA diff cannot be encoded.
pub fn compute_epoch_da<S>(
    spec: OLSpecId,
    pre_epoch_state: S,
    epoch_blocks: &[OLBlockV1],
    previous_terminal: &OLBlockHeaderV1,
    runtime_params: &OLRuntimeParams,
) -> Result<EpochReplayArtifacts<OLLog>, EpochDaError>
where
    S: IStateAccessorMut,
    DaAccumulatingState<S>: IStateAccessorMut,
{
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

    let mut da_state = DaAccumulatingState::new(pre_epoch_state);
    let ol_logs = execute_block_batch_predrain(
        spec,
        &mut da_state,
        epoch_blocks,
        previous_terminal,
        runtime_params,
    )
    .map_err(EpochDaError::BlockReplay)?;

    let encoded_da = da_state
        .take_completed_epoch_da_blob()
        .map_err(EpochDaError::Accumulation)?
        .expect("a successfully replayed terminal epoch must produce a DA blob");

    Ok(EpochReplayArtifacts {
        encoded_da,
        ol_logs,
    })
}

#[cfg(test)]
mod tests {
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_stf::{BlockComponents, OLSpecId};
    use strata_ol_stf_v1::test_utils::{
        epoch_runner_run_block as run_block, epoch_runner_run_genesis as run_genesis,
        make_genesis_state,
    };

    use super::{EpochDaError, compute_epoch_da};

    #[test]
    fn rejects_incomplete_epoch() {
        let mut state = make_genesis_state();
        let previous_terminal = run_genesis(&mut state);
        let pre_epoch_state = state.clone();
        let runtime_params = OLRuntimeParams::test_default();

        let empty_error = compute_epoch_da(
            OLSpecId::V1,
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
            OLSpecId::V1,
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
