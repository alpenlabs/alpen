//! Pure helpers for constructing OL genesis artifacts.

use std::result::Result as StdResult;

use strata_acct_types::AcctError;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Buf64, OLBlockCommitment};
use strata_ol_chain_types_v1::{OLBlockV1, SignedOLBlockHeaderV1};
use strata_ol_params::OLParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::StateError;
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, ExecError, OLSpecId, execute_and_complete_block,
};
use thiserror::Error;
use tracing::{info, instrument};

/// In-memory artifacts created during OL genesis construction.
#[derive(Debug)]
pub struct GenesisArtifacts {
    /// The OL state after executing the genesis block, with both spec
    /// versions at [`OLSpecId::GENESIS`](strata_ol_state_types::OLSpecId::GENESIS).
    pub ol_state: OLStateContainer,

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

    // Create initial OL state (uses genesis params). This is the only place a
    // state receives the genesis spec versions; every later state inherits
    // them from its parent.
    let mut ol_state = MemoryStateBaseLayer::new_genesis(params)?;

    // Create genesis block info.
    let genesis_ts = params.genesis_params().header().timestamp;
    let genesis_info = BlockInfo::new_genesis(genesis_ts);

    // Both in-state and DB-side MMRs are height-indexed: sentinel leaves occupy
    // indices 0..=genesis_l1_height, so the manifest for L1 height h lands at leaf h.
    // OL genesis must not append the genesis manifest or shift that index by one.
    // Genesis is the epoch terminal for epoch 0 (it carries no manifests, but
    // terminality is set explicitly via the header flag).
    let genesis_components = BlockComponents::new_manifests(vec![]).as_terminal();

    // Execute genesis block through the OL STF, under the same spec the
    // genesis state versions name.
    let block_context = BlockContext::new(&genesis_info, None);
    let runtime_params = params.runtime_params();
    let genesis_block = execute_and_complete_block(
        OLSpecId::GENESIS,
        &mut ol_state,
        block_context,
        genesis_components,
        &runtime_params,
    )?;
    let ol_state = ol_state.into_container();
    debug_assert_eq!(
        ol_state.compute_state_root(),
        *genesis_block.header().state_root(),
        "ol/genesis: container root must match the genesis header"
    );

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

#[cfg(test)]
mod tests {
    use strata_ol_state_types::{OLRootState, OLSpecId};

    use super::*;

    /// Executed genesis state root for [`OLParams::test_default`].
    ///
    /// This changes whenever the root state layout, the V1 chainstate layout,
    /// genesis versions, or genesis execution change, all of which require a
    /// network reset.
    const TEST_GENESIS_STATE_ROOT: &str =
        "3ed8ee16d5157b272843cae6a22c6d8de6516dc34d81b04aabfa00d1d26e80aa";

    /// Genesis block ID for [`OLParams::test_default`], which commits to
    /// [`TEST_GENESIS_STATE_ROOT`].
    const TEST_GENESIS_BLKID: &str =
        "84ed81e8751d3194bb51221446e11bfcd58fd65e92873821ca90e221cb850a7e";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn test_genesis_commits_to_root_state() {
        let artifacts = build_genesis_artifacts(&OLParams::test_default()).unwrap();
        let header_root = *artifacts.ol_block.header().state_root();

        let genesis_version = u32::from(OLSpecId::GENESIS);
        let chainstate_root = artifacts.ol_state.chainstate().compute_chainstate_root();
        assert_eq!(
            header_root,
            OLRootState::new(genesis_version, genesis_version, chainstate_root)
                .compute_state_root()
        );
        assert_eq!(artifacts.epoch_summary.final_state(), &header_root);

        assert_eq!(hex(header_root.as_ref()), TEST_GENESIS_STATE_ROOT);
        assert_eq!(
            hex(artifacts.commitment.blkid().as_ref()),
            TEST_GENESIS_BLKID
        );
    }
}
