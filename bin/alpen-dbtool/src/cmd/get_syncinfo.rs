use std::sync::Arc;

use strata_db::traits::{
    BlockStatus, ChainstateDatabase, ClientStateDatabase, Database, L1Database, L2BlockDatabase,
    SyncEventDatabase,
};
use strata_primitives::{l1::L1BlockId, l2::L2BlockId};
use strata_rocksdb::CommonDb;

use crate::{
    cmd::GetSyncinfo,
    errors::{DbtoolError, Result},
};

#[derive(Debug)]
pub struct SyncInfo {
    /* -------- L1 -------- */
    pub l1_tip_height: Option<u64>,
    pub l1_tip_blkid: Option<L1BlockId>,

    /* -------- L2 -------- */
    pub l2_head_height: Option<u64>,
    pub l2_head_id: Option<L2BlockId>,
    pub l2_head_status: Option<BlockStatus>,

    /* ----- pipeline ----- */
    pub last_sync_event: Option<u64>,
    pub last_client_state: Option<u64>,
}

pub fn get_syncinfo(db: Arc<CommonDb>, _args: GetSyncinfo) -> Result<()> {
    let (l1_tip_height, l1_tip_blkid) = db
        .l1_db()
        .get_canonical_chain_tip()
        .map_err(|e| DbtoolError::Db(e.to_string()))?
        .unwrap_or_default();

    /* --- L2 --- */
    let l2_head_height = db
        .chain_state_db()
        .get_last_write_idx()
        .map_err(|e| DbtoolError::Db(e.to_string()))
        .ok(); // treat errors as “unknown”

    let l2_head_id = l2_head_height
        .and_then(|h| db.l2_db().get_blocks_at_height(h).ok())
        .and_then(|mut v| v.pop());

    let l2_head_status = l2_head_id
        .and_then(|id| db.l2_db().get_block_status(id).ok())
        .flatten();

    /* --- pipeline --- */
    let last_sync_event = db.sync_event_db().get_last_idx().ok().flatten();
    let last_client_state = db.client_state_db().get_last_state_idx().ok();

    let sync_info = SyncInfo {
        l1_tip_height: Some(l1_tip_height),
        l1_tip_blkid: Some(l1_tip_blkid),
        l2_head_height,
        l2_head_id,
        l2_head_status,
        last_sync_event,
        last_client_state,
    };

    println!(
        "{} {}/{}",
        "L1 tip:",
        sync_info.l1_tip_height.unwrap_or_default(),
        sync_info
            .l1_tip_blkid
            .map(|h| format!("{h:?}"))
            .unwrap_or_default()
    );
    println!(
        "{} {}/{}  ({:?})",
        "L2 head:",
        sync_info.l2_head_height.unwrap_or_default(),
        sync_info
            .l2_head_id
            .map(|h| format!("{h:?}"))
            .unwrap_or_default(),
        sync_info.l2_head_status.unwrap_or(BlockStatus::Unchecked),
    );
    println!(
        "{} {:?} | {} {:?}",
        "sync-event idx:",
        sync_info.last_sync_event,
        "client-state idx:",
        sync_info.last_client_state
    );

    Ok(())
}
