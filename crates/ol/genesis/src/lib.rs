//! Pure helpers for constructing OL genesis artifacts.

use std::result::Result as StdResult;

use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Buf64, L1BlockCommitment, OLBlockCommitment};
use strata_ledger_types::AsmManifest;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_state_types::OLState;
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, ExecError, execute_and_complete_block,
};
use strata_params::Params;
use strata_primitives::Buf32;
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
}

pub type Result<T> = StdResult<T, GenesisError>;

/// Build the default genesis manifest from rollup params.
pub fn default_genesis_manifest(params: &Params) -> AsmManifest {
    let genesis_l1 = &params.rollup().genesis_l1_view;

    AsmManifest::new(
        genesis_l1.height_u64(),
        genesis_l1.blkid(),
        // TODO: Properly fetch manifest from db and populate this, btc reader should read L1 and
        // send events/msgs to asm worker for this to be correctly done.
        Buf32::zero().into(),
        vec![],
    )
}

/// Construct genesis state + block artifacts using a supplied manifest.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub fn build_genesis_artifacts_with_manifest(
    params: &Params,
    genesis_manifest: AsmManifest,
) -> Result<GenesisArtifacts> {
    info!("building OL genesis block and state");

    // Create initial OL state (uses genesis defaults).
    // TODO: initialize with a Snark EE account for Alpen. Possibly with rollup params.
    let mut ol_state = OLState::new_genesis();

    // Create genesis block info.
    let genesis_l1 = &params.rollup().genesis_l1_view;
    let genesis_ts = genesis_l1.last_11_timestamps[10] as u64;
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
    let genesis_l1_commitment =
        L1BlockCommitment::from_height_u64(genesis_l1.height_u64(), genesis_l1.blkid()).ok_or(
            GenesisError::InvalidGenesisL1Height {
                height: genesis_l1.height_u64(),
            },
        )?;
    let epoch_summary = EpochSummary::new(
        0,
        commitment,
        OLBlockCommitment::null(),
        genesis_l1_commitment,
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
pub fn build_genesis_artifacts(params: &Params) -> Result<GenesisArtifacts> {
    let genesis_manifest = default_genesis_manifest(params);
    build_genesis_artifacts_with_manifest(params, genesis_manifest)
}
