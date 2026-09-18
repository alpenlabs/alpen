//! Pure helpers for constructing OL genesis artifacts.

use std::result::Result as StdResult;

use strata_acct_types::AcctError;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Buf64, OLBlockCommitment};
use strata_ol_chain_types_v1::{OLBlockV1, SignedOLBlockHeaderV1};
use strata_ol_params::OLParams;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::StateError;
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf_v1::{
    BlockComponents, BlockContext, BlockInfo, ExecError, execute_and_complete_block,
};
use thiserror::Error;
use tracing::{info, instrument};

/// In-memory artifacts created during OL genesis construction.
#[derive(Debug)]
pub struct GenesisArtifacts {
    /// The initial OL state.
    pub ol_state: OLStateV1,

    /// The genesis OL block.
    pub ol_block: OLBlockV1,

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

    /// Account related errors.
    #[error("acct: {0}")]
    Acct(#[from] AcctError),

    #[error("state: {0}")]
    State(#[from] StateError),
}

pub type Result<T> = StdResult<T, GenesisError>;

/// Constructs the genesis OL state and block artifacts from the given parameters.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub fn build_genesis_artifacts(params: &OLParams) -> Result<GenesisArtifacts> {
    info!("building OL genesis block and state");

    // Create initial OL state (uses genesis params).
    let ol_state_raw = OLStateV1::from_genesis_params(params)?;
    let mut ol_state = MemoryStateBaseLayer::new(ol_state_raw);

    // Create genesis block info.
    let genesis_ts = params.genesis_params().header().timestamp;
    let genesis_info = BlockInfo::new_genesis(genesis_ts);

    // Both in-state and DB-side MMRs are height-indexed: sentinel leaves occupy
    // indices 0..=genesis_l1_height, so the manifest for L1 height h lands at leaf h.
    // OL genesis must not append the genesis manifest or shift that index by one.
    // Genesis is the epoch terminal for epoch 0 (it carries no manifests, but
    // terminality is set explicitly via the header flag).
    let genesis_components = BlockComponents::new_manifests(vec![]).as_terminal();

    // Execute genesis block through the OL STF.
    let block_context = BlockContext::new(&genesis_info, None);
    let runtime_params = params.runtime_params();
    let genesis_block = execute_and_complete_block(
        &mut ol_state,
        block_context,
        genesis_components,
        &runtime_params,
    )?;
    let ol_state = ol_state.into_inner();

    // Create signed header (genesis uses zero signature).
    let signed_header = SignedOLBlockHeaderV1::new(genesis_block.header().clone(), Buf64::zero());
    let ol_block = OLBlockV1::new(signed_header, genesis_block.body().clone());
    let genesis_blkid = genesis_block.header().compute_blkid();
    let commitment = OLBlockCommitment::new(0, genesis_blkid);

    let epoch_summary = EpochSummary::new(
        0,
        commitment,
        OLBlockCommitment::null(),
        params.genesis_l1_block(),
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
