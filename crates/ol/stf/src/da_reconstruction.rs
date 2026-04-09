//! DA-based epoch state reconstruction.
//!
//! Provides the canonical 3-step pipeline for reconstructing epoch state from a
//! DA payload: epoch initialization, state diff application, and manifest
//! processing.

use strata_ledger_types::{
    IAccountState, IAccountStateConstructible, ISnarkAccountStateConstructible, IStateAccessor,
};
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_da::{DaScheme, OLDaPayloadV1, OLDaSchemeV1};

use crate::{
    BasicExecContext, BlockInfo, EpochInitialContext, ExecOutputBuffer, errors::ExecResult,
    process_block_manifests, process_epoch_initial,
};

/// Reconstructs epoch state from a DA payload.
///
/// Runs the 3-steps: epoch initialization, DA state diff application,
/// and ASM manifest processing. The caller is responsible for wrapping `state`
/// in any tracking layers (e.g. `IndexerState`) before calling this.
pub fn apply_da_epoch<S>(
    epctx: &EpochInitialContext,
    state: &mut S,
    payload: OLDaPayloadV1,
    terminal_blkinfo: BlockInfo,
    manifests: Option<&OLL1ManifestContainer>,
) -> ExecResult<()>
where
    S: IStateAccessor,
    S::AccountState: IAccountStateConstructible,
    <S::AccountState as IAccountState>::SnarkAccountState: ISnarkAccountStateConstructible,
{
    process_epoch_initial(state, epctx)?;

    OLDaSchemeV1::apply_to_state(payload, state)?;

    if let Some(mf) = manifests {
        let outbuf = ExecOutputBuffer::new_empty();
        let exctx = BasicExecContext::new(terminal_blkinfo, &outbuf);
        process_block_manifests(state, mf, &exctx)?;
    }

    Ok(())
}
