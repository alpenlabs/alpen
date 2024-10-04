use strata_db::{errors::DbError, traits::*};
use strata_primitives::{
    buf::{Buf32, Buf64},
    evm_exec::create_evm_extra_payload,
    l1::L1BlockManifest,
    params::{OperatorConfig, Params},
};
use strata_state::{
    block::{ExecSegment, L1Segment, L2BlockAccessory, L2BlockBundle},
    bridge_state::OperatorTable,
    chain_state::ChainState,
    client_state::ClientState,
    exec_env::ExecEnvState,
    exec_update::{ExecUpdate, UpdateInput, UpdateOutput},
    genesis::GenesisStateData,
    header::L2BlockHeader,
    l1::{L1HeaderRecord, L1ViewState},
    prelude::*,
};
use tracing::*;

use crate::errors::Error;

/// Inserts into the database an initial basic client state that we can begin
/// waiting for genesis with.
pub fn init_client_state(params: &Params, database: &impl Database) -> anyhow::Result<()> {
    debug!("initializing client state in database!");

    let init_state = ClientState::from_genesis_params(
        params.rollup().horizon_l1_height,
        params.rollup().genesis_l1_height,
    );

    // Write the state into the database.
    let cs_store = database.client_state_store();
    cs_store.write_client_state_checkpoint(0, init_state)?;

    Ok(())
}

/// Inserts appropriate chainstate into the database to start actively syncing
/// the rollup chain.  Requires that the L1 blocks between the horizon and the
/// L2 genesis are already in the datatabase.
///
/// This does not update the client state to include the new sync state data
/// that it should have now.  That is introduced by writing a new sync event for
/// that.
pub fn init_genesis_chainstate(
    params: &Params,
    database: &impl Database,
) -> anyhow::Result<L2BlockId> {
    debug!("preparing database genesis chainstate!");

    let horizon_blk_height = params.rollup.horizon_l1_height;
    let genesis_blk_height = params.rollup.genesis_l1_height;

    // Query the pre-genesis blocks we need before we do anything else.
    let l1_prov = database.l1_provider();
    let pregenesis_mfs =
        load_pre_genesis_l1_manifests(l1_prov.as_ref(), horizon_blk_height, genesis_blk_height)?;

    // Build the genesis block and genesis consensus states.
    let gblock = make_genesis_block(params);
    let gchstate = make_genesis_chainstate(&gblock, pregenesis_mfs, params);

    let genesis_blkid = gblock.header().get_blockid();

    // Now insert things into the database.
    let chs_store = database.chain_state_store();
    let l2store = database.l2_store();
    chs_store.write_genesis_state(&gchstate)?;
    l2store.put_block_data(gblock)?;

    // TODO make ^this be atomic so we can't accidentally not write both, or
    // make it so we can overwrite the genesis chainstate if there's no other
    // states or something

    info!("finished genesis insertions");
    Ok(genesis_blkid)
}

pub fn construct_operator_table(opconfig: &OperatorConfig) -> OperatorTable {
    match opconfig {
        OperatorConfig::Static(oplist) => OperatorTable::from_operator_list(oplist),
    }
}

fn load_pre_genesis_l1_manifests(
    l1_prov: &impl L1DataProvider,
    horizon_height: u64,
    genesis_height: u64,
) -> anyhow::Result<Vec<L1BlockManifest>> {
    let mut manifests = Vec::new();
    for height in horizon_height..=genesis_height {
        let Some(mf) = l1_prov.get_block_manifest(height)? else {
            return Err(Error::MissingL1BlockHeight(height).into());
        };

        manifests.push(mf);
    }

    Ok(manifests)
}

/// Create genesis L2 block based on rollup params
/// NOTE: generate block MUST be deterministic
/// repeated calls with same params MUST return identical blocks
pub fn make_genesis_block(params: &Params) -> L2BlockBundle {
    // Create a dummy exec state that we can build the rest of the genesis block
    // around and insert into the genesis state.
    // TODO this might need to talk to the EL to do the genesus setup *properly*
    let extra_payload = create_evm_extra_payload(params.rollup.evm_genesis_block_hash);
    let geui = UpdateInput::new(0, vec![], Buf32::zero(), extra_payload);
    let genesis_update = ExecUpdate::new(
        geui.clone(),
        UpdateOutput::new_from_state(params.rollup.evm_genesis_block_state_root),
    );

    // This has to be empty since everyone should have an unambiguous view of the genesis block.
    let l1_seg = L1Segment::new_empty();

    // TODO this is a total stub, we have to fill it in with something
    let exec_seg = ExecSegment::new(genesis_update);

    let body = L2BlockBody::new(l1_seg, exec_seg);

    // TODO stub
    let exec_payload = vec![];
    let accessory = L2BlockAccessory::new(exec_payload);

    // Assemble the genesis header template, pulling in data from whatever
    // sources we need.
    // FIXME this isn't the right timestamp to start the blockchain, this should
    // definitely be pulled from the database or the rollup parameters maybe
    let genesis_ts = params.rollup().horizon_l1_height;
    let zero_blkid = L2BlockId::from(Buf32::zero());
    let genesis_sr = Buf32::zero();
    let header = L2BlockHeader::new(0, genesis_ts, zero_blkid, &body, genesis_sr);
    let signed_genesis_header = SignedL2BlockHeader::new(header, Buf64::zero());
    let block = L2Block::new(signed_genesis_header, body);
    L2BlockBundle::new(block, accessory)
}

pub fn make_genesis_chainstate(
    gblock: &L2BlockBundle,
    pregenesis_mfs: Vec<L1BlockManifest>,
    params: &Params,
) -> ChainState {
    let genesis_blkid = gblock.header().get_blockid();

    let geui = gblock.exec_segment().update().input();
    let gees =
        ExecEnvState::from_base_input(geui.clone(), params.rollup.evm_genesis_block_state_root);

    let horizon_blk_height = params.rollup.horizon_l1_height;
    let genesis_blk_rec = L1HeaderRecord::from(pregenesis_mfs.last().unwrap());
    let l1vs = L1ViewState::new_at_horizon(horizon_blk_height, genesis_blk_rec);

    let optbl = construct_operator_table(&params.rollup().operator_config);
    let gdata = GenesisStateData::new(genesis_blkid, l1vs, optbl, gees);
    ChainState::from_genesis(&gdata)
}

/// Check if the database needs to have client init done to it.
pub fn check_needs_client_init(database: &impl Database) -> anyhow::Result<bool> {
    let cs_prov = database.client_state_provider();

    // Check if we've written any genesis state checkpoint.  These we perform a
    // bit more carefully and check errors more granularly.
    match cs_prov.get_last_checkpoint_idx() {
        Ok(_) => {}
        Err(DbError::NotBootstrapped) => return Ok(true),

        // TODO should we return an error here or skip?
        Err(e) => return Err(e.into()),
    }

    Ok(false)
}

pub fn check_needs_genesis(database: &impl Database) -> anyhow::Result<bool> {
    let l2_prov = database.l2_provider();

    // Check if there's any genesis block written.
    match l2_prov.get_blocks_at_height(0) {
        Ok(blkids) => Ok(blkids.is_empty()),

        Err(DbError::NotBootstrapped) => Ok(true),

        // Again, how should we handle this?
        Err(e) => Err(e.into()),
    }
}
