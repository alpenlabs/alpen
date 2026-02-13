//! Pure helpers for constructing OL genesis artifacts.

use std::result::Result as StdResult;

use strata_acct_types::{AcctError, BitcoinAmount};
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{AccountId, Buf32, Buf64, L1BlockCommitment, OLBlockCommitment};
use strata_ledger_types::AsmManifest;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_params::{AccountParams, OLParams};
use strata_ol_state_types::OLState;
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, ExecError, execute_and_complete_block,
};
use strata_params::Params;
use strata_predicate::PredicateKey;
use thiserror::Error;
use tracing::{info, instrument};

/// 32-byte representation of the [`AccountId`] for the Alpen EE account.
// TODO: this should be decided by the product team if they want to have
//       a special number for the account id. The only restriction is that
//       it should not be a special account id, i.e. in the range of 0-127.
pub const ALPEN_EE_ACCOUNT_ID_BYTES: [u8; 32] = [1; 32];

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
pub fn default_genesis_manifest(params: &Params) -> AsmManifest {
    let genesis_l1 = &params.rollup().genesis_l1_view;

    AsmManifest::new(
        genesis_l1.height_u64(),
        genesis_l1.blkid(),
        // Placeholder manifest root for non-ASM genesis (tests/fallback paths).
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

    // Create genesis block info.
    let genesis_l1 = &params.rollup().genesis_l1_view;
    let genesis_l1_commitment =
        L1BlockCommitment::from_height_u64(genesis_l1.height_u64(), genesis_l1.blkid()).ok_or(
            GenesisError::InvalidGenesisL1Height {
                height: genesis_l1.height_u64(),
            },
        )?;

    let mut ol_params = OLParams {
        last_l1_block: genesis_l1_commitment,
        ..OLParams::default()
    };

    let alpen_ed_account = AccountId::new(ALPEN_EE_ACCOUNT_ID_BYTES);
    let alpen_ed_state_root = params.rollup().evm_genesis_block_state_root;
    ol_params
        .accounts
        .entry(alpen_ed_account)
        .or_insert_with(|| AccountParams {
            predicate: PredicateKey::always_accept(),
            inner_state: alpen_ed_state_root,
            balance: BitcoinAmount::ZERO,
        });

    // Create initial OL state (uses genesis params).
    let mut ol_state = OLState::from_genesis_params(&ol_params)?;

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
