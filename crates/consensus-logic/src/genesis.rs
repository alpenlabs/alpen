use strata_db::errors::DbError;
use strata_primitives::{
    buf::{Buf32, Buf64},
    evm_exec::create_evm_extra_payload,
    l1::L1BlockManifest,
    params::{OperatorConfig, Params},
};
use strata_state::{
    block::{ExecSegment, L1Segment, L2BlockAccessory, L2BlockBundle},
    bridge_state::OperatorTable,
    chain_state::Chainstate,
    client_state::ClientState,
    exec_env::ExecEnvState,
    exec_update::{ExecUpdate, UpdateInput, UpdateOutput},
    genesis::GenesisStateData,
    header::L2BlockHeader,
    l1::L1ViewState,
    operation::ClientUpdateOutput,
    prelude::*,
};
use strata_storage::{ClientStateManager, L1BlockManager, L2BlockManager, NodeStorage};
use tracing::*;

use crate::errors::Error;

/// Inserts into the database an initial basic client state that we can begin
/// waiting for genesis with.
pub fn init_client_state(params: &Params, csman: &ClientStateManager) -> anyhow::Result<()> {
    debug!("initializing client state in database!");

    let init_state = ClientState::from_genesis_params(params);

    // Write the state into the database.
    csman.put_update_blocking(0, ClientUpdateOutput::new_state(init_state))?;
    // TODO: status channel should probably be updated.

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
    storage: &NodeStorage,
) -> anyhow::Result<Chainstate> {
    debug!("preparing database genesis chainstate!");

    let horizon_blk_height = params.rollup.horizon_l1_height;
    let genesis_blk_height = params.rollup.genesis_l1_height;

    // Query the pre-genesis blocks we need before we do anything else.
    let l1_db = storage.l1();
    let pregenesis_mfs =
        load_pre_genesis_l1_manifests(l1_db.as_ref(), horizon_blk_height, genesis_blk_height)?;

    // Build the genesis block and genesis consensus states.
    let (gblock, gchstate) = make_l2_genesis(params, pregenesis_mfs);

    // Now insert things into the database.
    storage
        .chainstate()
        .write_genesis_state(gchstate.clone(), gblock.header().get_blockid())?;
    storage.l2().put_block_data_blocking(gblock)?;
    // TODO: Status channel should probably be updated.

    // TODO make ^this be atomic so we can't accidentally not write both, or
    // make it so we can overwrite the genesis chainstate if there's no other
    // states or something

    info!("finished genesis insertions");
    Ok(gchstate)
}

pub fn construct_operator_table(opconfig: &OperatorConfig) -> OperatorTable {
    match opconfig {
        OperatorConfig::Static(oplist) => OperatorTable::from_operator_list(oplist),
    }
}

fn load_pre_genesis_l1_manifests(
    l1man: &L1BlockManager,
    horizon_height: u64,
    genesis_height: u64,
) -> anyhow::Result<Vec<L1BlockManifest>> {
    let mut manifests = Vec::new();
    for height in horizon_height..=genesis_height {
        let Some(mf) = l1man.get_block_manifest_at_height(height)? else {
            return Err(Error::MissingL1BlockHeight(height).into());
        };

        manifests.push(mf);
    }

    Ok(manifests)
}

pub fn make_l2_genesis(
    params: &Params,
    pregenesis_mfs: Vec<L1BlockManifest>,
) -> (L2BlockBundle, Chainstate) {
    let gblock_provisional = make_genesis_block(params);
    let gstate = make_genesis_chainstate(&gblock_provisional, pregenesis_mfs, params);
    let state_root = gstate.compute_state_root();

    let (block, accessory) = gblock_provisional.into_parts();
    let (header, body) = block.into_parts();

    let final_header = L2BlockHeader::new(
        header.slot(),
        header.epoch(),
        header.timestamp(),
        *header.parent(),
        &body,
        state_root,
    );
    let sig = Buf64::zero();
    let gblock = L2BlockBundle::new(
        L2Block::new(SignedL2BlockHeader::new(final_header, sig), body),
        accessory,
    );

    (gblock, gstate)
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
    let l1_seg = L1Segment::new_empty(params.rollup().genesis_l1_height);

    // TODO this is a total stub, we have to fill it in with something
    let exec_seg = ExecSegment::new(genesis_update);

    let body = L2BlockBody::new(l1_seg, exec_seg);

    // TODO stub
    let exec_payload = vec![];
    let accessory = L2BlockAccessory::new(exec_payload, 0);

    // Assemble the genesis header template, pulling in data from whatever
    // sources we need.
    // FIXME this isn't the right timestamp to start the blockchain, this should
    // definitely be pulled from the database or the rollup parameters maybe
    let genesis_ts = params.rollup().horizon_l1_height;
    let zero_blkid = L2BlockId::from(Buf32::zero());
    let genesis_sr = Buf32::zero();
    let header = L2BlockHeader::new(0, 0, genesis_ts, zero_blkid, &body, genesis_sr);
    let signed_genesis_header = SignedL2BlockHeader::new(header, Buf64::zero());
    let block = L2Block::new(signed_genesis_header, body);
    L2BlockBundle::new(block, accessory)
}

fn make_genesis_chainstate(
    gblock: &L2BlockBundle,
    pregenesis_mfs: Vec<L1BlockManifest>,
    params: &Params,
) -> Chainstate {
    let geui = gblock.exec_segment().update().input();
    let gees =
        ExecEnvState::from_base_input(geui.clone(), params.rollup.evm_genesis_block_state_root);

    let horizon_blk_height = params.rollup.horizon_l1_height;
    let genesis_blk_height = params.rollup.genesis_l1_height;
    let genesis_mf = pregenesis_mfs
        .last()
        .expect("genesis block must be present")
        .clone();
    let gheader_vs = genesis_mf
        .header_verification_state()
        .as_ref()
        .expect("genesis block must have HeaderVS")
        .clone();
    let genesis_blk_rec = genesis_mf.record().clone();
    let l1vs = L1ViewState::new_at_genesis(
        horizon_blk_height,
        genesis_blk_height,
        genesis_blk_rec,
        gheader_vs,
    );

    let optbl = construct_operator_table(&params.rollup().operator_config);
    let gdata = GenesisStateData::new(l1vs, optbl, gees);
    Chainstate::from_genesis(&gdata)
}

/// Check if the database needs to have client init done to it.
pub fn check_needs_client_init(storage: &NodeStorage) -> anyhow::Result<bool> {
    // Check if we've written any genesis state checkpoint.  These we perform a
    // bit more carefully and check errors more granularly.
    match storage.client_state().get_last_state_idx_blocking() {
        Ok(_) => {}
        Err(DbError::NotBootstrapped) => return Ok(true),

        // TODO should we return an error here or skip?
        Err(e) => return Err(e.into()),
    }

    Ok(false)
}

/// Checks if we have a genesis block written to the L2 block database.
pub fn check_needs_genesis(l2man: &L2BlockManager) -> anyhow::Result<bool> {
    // Check if there's any genesis block written.
    match l2man.get_blocks_at_height_blocking(0) {
        Ok(blkids) => Ok(blkids.is_empty()),

        Err(DbError::NotBootstrapped) => Ok(true),

        // Again, how should we handle this?
        Err(e) => Err(e.into()),
    }
}
