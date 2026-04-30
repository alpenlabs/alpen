//! Pure helpers for constructing OL genesis artifacts.

use std::result::Result as StdResult;

use strata_acct_types::AcctError;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Buf64, OLBlockCommitment};
use strata_ledger_types::StateError;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_params::OLParams;
use strata_ol_state_support_types::MemoryStateBaseLayer;
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
    let ol_state_raw = OLState::from_genesis_params(params)?;
    let mut ol_state = MemoryStateBaseLayer::new(ol_state_raw);

    // Create genesis block info.
    let genesis_ts = params.header.timestamp;
    let genesis_info = BlockInfo::new_genesis(genesis_ts);

    // Do not include the genesis manifest in OL state's ASM manifest accumulator.
    //
    // ASM worker stores genesis manifest for data consumers, but intentionally starts the
    // external/global ASM MMR at `genesis_l1_height + 1` (first post-genesis manifest at leaf 0).
    // If OL genesis appends the genesis manifest here, OL state's MMR gets an extra leading leaf
    // and ledger-reference proofs become permanently off-by-one against the global ASM MMR.
    let genesis_components = BlockComponents::new_manifests(vec![]);

    // Execute genesis block through the OL STF.
    let block_context = BlockContext::new(&genesis_info, None);
    let genesis_block =
        execute_and_complete_block(&mut ol_state, block_context, genesis_components)?;
    let ol_state = ol_state.into_inner();

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
