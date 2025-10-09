use strata_asm_types::HeaderVerificationState;
use strata_db::errors::DbError;
use strata_ol_chain_types::{
    ExecSegment, L1Segment, L2Block, L2BlockAccessory, L2BlockBody, L2BlockBundle, L2BlockHeader,
    L2Header, SignedL2BlockHeader,
};
use strata_ol_chainstate_types::{Chainstate, GenesisStateData, L1ViewState, WriteBatch};
use strata_btc_types::constants::TIMESTAMPS_FOR_MEDIAN;
use strata_identifiers::{Buf32, Buf64};
use strata_params::{OperatorConfig, Params};
use strata_primitives::evm_exec::create_evm_extra_payload;
use strata_ol_chain_types::{ExecUpdate, UpdateInput, UpdateOutput};
use strata_state::{
    bridge_state::OperatorTable,
    client_state::ClientState,
    exec_env::ExecEnvState,
    operation::ClientUpdateOutput,
    prelude::*,
};
use strata_storage::{L2BlockManager, NodeStorage};
use tokio::runtime::Handle;
use tracing::*;

/// Inserts into the database an initial basic client state that we can begin
/// waiting for genesis with.
pub fn init_client_state(params: &Params, storage: &NodeStorage) -> anyhow::Result<()> {
    debug!("initializing client state in database!");

    let init_state = ClientState::default();
    init_genesis_chainstate(params, storage)?;

    // Write the state into the database.
    storage.client_state().put_update_blocking(
        &params.rollup().genesis_l1_view.blk,
        ClientUpdateOutput::new_state(init_state),
    )?;

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
) -> anyhow::Result<(L2BlockId, Chainstate)> {
    debug!("preparing database genesis chainstate!");

    // Build the genesis block and genesis consensus states.
    let (gblock, gchstate) = make_l2_genesis(params);

    // Now insert things into the database.
    let gid = gblock.header().get_blockid();

    let wb = WriteBatch::new(gchstate.clone());
    storage
        .chainstate()
        .put_slot_write_batch_blocking(gid, wb)?;
    storage.l2().put_block_data_blocking(gblock)?;
    storage
        .l2()
        .set_block_status_blocking(&gid, strata_db::traits::BlockStatus::Valid)?;
    // TODO: Status channel should probably be updated.

    // TODO make ^this be atomic so we can't accidentally not write both, or
    // make it so we can overwrite the genesis chainstate if there's no other
    // states or something

    info!("finished genesis insertions");
    Ok((gid, gchstate))
}

pub fn construct_operator_table(opconfig: &OperatorConfig) -> OperatorTable {
    match opconfig {
        OperatorConfig::Static(oplist) => OperatorTable::from_operator_list(oplist),
    }
}

pub fn make_l2_genesis(params: &Params) -> (L2BlockBundle, Chainstate) {
    let gblock_provisional = make_genesis_block(params);
    let gstate = make_genesis_chainstate(&gblock_provisional, params);
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
    let l1_seg = L1Segment::new_empty(params.rollup().genesis_l1_view.blk.height_u64());

    // TODO this is a total stub, we have to fill it in with something
    let exec_seg = ExecSegment::new(genesis_update);

    let body = L2BlockBody::new(l1_seg, exec_seg);

    // TODO stub
    let exec_payload = vec![];
    let accessory = L2BlockAccessory::new(exec_payload, 0);

    let genesis_ts =
        params.rollup().genesis_l1_view.last_11_timestamps[TIMESTAMPS_FOR_MEDIAN - 1] as u64;
    let zero_blkid = L2BlockId::from(Buf32::zero());
    let genesis_sr = Buf32::zero();
    let header = L2BlockHeader::new(0, 0, genesis_ts, zero_blkid, &body, genesis_sr);
    let signed_genesis_header = SignedL2BlockHeader::new(header, Buf64::zero());
    let block = L2Block::new(signed_genesis_header, body);
    L2BlockBundle::new(block, accessory)
}

fn make_genesis_chainstate(gblock: &L2BlockBundle, params: &Params) -> Chainstate {
    let geui = gblock.exec_segment().update().input();
    let gees =
        ExecEnvState::from_base_input(geui.clone(), params.rollup.evm_genesis_block_state_root);

    let genesis_l1_view = &params.rollup().genesis_l1_view;
    let gheader_vs = HeaderVerificationState::new(params.network(), genesis_l1_view);
    let l1vs = L1ViewState::new_at_genesis(genesis_l1_view.height_u64(), gheader_vs);

    let optbl = construct_operator_table(&params.rollup().operator_config);
    let gdata = GenesisStateData::new(l1vs, optbl, gees);
    Chainstate::from_genesis(&gdata)
}

/// Check if the database needs to have client init done to it.
pub fn check_needs_client_init(storage: &NodeStorage) -> anyhow::Result<bool> {
    // Check if we've written any pre-genesis client state.
    Ok(storage.client_state().fetch_most_recent_state()?.is_none())
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

pub fn wait_for_genesis<T>(f: impl Fn() -> Option<L2BlockId>, handle: Handle) -> L2BlockId {
    let genesis_block_id = handle.block_on(async {
        while f().is_none() {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
        f().expect("genesis should be in")
    });
    genesis_block_id
}
