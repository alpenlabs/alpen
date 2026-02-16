//! Pure helpers for constructing OL genesis artifacts.

use std::result::Result as StdResult;

use strata_acct_types::AcctError;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Buf32, Buf64, OLBlockCommitment};
use strata_ledger_types::AsmManifest;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_params::OLParams;
use strata_ol_state_types::OLState;
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, ExecError, execute_and_complete_block,
};
use thiserror::Error;
use tracing::{info, instrument};

/// In-memory artifacts created during OL genesis construction.
#[derive(Debug)]
pub struct GenesisArtifacts {
    /// The initial OL state.
    pub ol_state: OLState,

    /// The genesis OL block.
    pub ol_block: OLBlock,

    /// The commitment to the genesis OL block.
    pub commitment: OLBlockCommitment,

    /// The epoch 0 summary for initializing checkpoint tracking.
    pub epoch_summary: EpochSummary,
}

/// Errors returned while building OL genesis artifacts.
#[derive(Debug, Error)]
pub enum GenesisError {
    /// The OL STF execution failed.
    #[error("OL STF execution failed")]
    StfExecution(#[from] ExecError),

    /// The genesis L1 height is invalid.
    #[error("invalid genesis L1 height {height}")]
    InvalidGenesisL1Height { height: u64 },

    /// Failed to construct the genesis OL state.
    #[error("failed to construct OL genesis state")]
    GenesisState(#[from] AcctError),
}

pub type Result<T> = StdResult<T, GenesisError>;

/// Build the default genesis manifest from rollup params.
pub fn default_genesis_manifest(params: &OLParams) -> AsmManifest {
    AsmManifest::new(
        params.last_l1_block.height_u64(),
        *params.last_l1_block.blkid(),
        // Placeholder manifest root for non-ASM genesis (tests/fallback paths).
        Buf32::zero().into(),
        vec![],
    )
}

/// Construct genesis state + block artifacts using a supplied manifest.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub fn build_genesis_artifacts_with_manifest(
    params: &OLParams,
    genesis_manifest: AsmManifest,
) -> Result<GenesisArtifacts> {
    info!("building OL genesis block and state");

    // Create initial OL state (uses genesis params).
    let mut ol_state = OLState::from_genesis_params(params)?;

    // Create genesis block info.
    let genesis_ts = params.header.timestamp;
    let genesis_info = BlockInfo::new_genesis(genesis_ts);

    // Build genesis block components.
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);

    // Execute genesis block through the OL STF.
    let block_context = BlockContext::new(&genesis_info, None);
    let genesis_block =
        execute_and_complete_block(&mut ol_state, block_context, genesis_components)?;

    // Create signed header (genesis uses zero signature).
    let signed_header = SignedOLBlockHeader::new(genesis_block.header().clone(), Buf64::zero());
    let ol_block = OLBlock::new(signed_header, genesis_block.body().clone());
    let genesis_blkid = genesis_block.header().compute_blkid();
    let commitment = OLBlockCommitment::new(0, genesis_blkid);

    let epoch_summary = EpochSummary::new(
        0,
        commitment,
        OLBlockCommitment::null(),
        params.last_l1_block,
        *genesis_block.header().state_root(),
    );

    info!(%genesis_blkid, slot = 0, "OL genesis build complete");

    Ok(GenesisArtifacts {
        ol_state,
        ol_block,
        commitment,
        epoch_summary,
    })
}

/// Construct genesis state + block artifacts using the default manifest.
pub fn build_genesis_artifacts(params: &OLParams) -> Result<GenesisArtifacts> {
    let genesis_manifest = default_genesis_manifest(params);
    build_genesis_artifacts_with_manifest(params, genesis_manifest)
}
