//! Bridge Msg operation interface

use std::sync::Arc;

use strata_db::interfaces::bridge_relay::BridgeMessageDb;
use strata_primitives::relay::types::BridgeMessage;

use crate::exec::*;

/// Database context for an database operation interface.
#[derive(Debug)]
pub struct Context<D: BridgeMessageDb> {
    db: Arc<D>,
}

impl<D: BridgeMessageDb + Sync + Send + 'static> Context<D> {
    pub const fn new(db: Arc<D>) -> Self {
        Self { db }
    }

    pub fn into_ops(self, pool: threadpool::ThreadPool) -> BridgeMsgOps {
        BridgeMsgOps::new(pool, Arc::new(self))
    }
}

inst_ops! {
    (BridgeMsgOps, Context<D: BridgeMessageDb>) {
         write_msg(id: u128, msg: BridgeMessage) => ();
         delete_msgs_before_timestamp(msg_ids: u128) => ();
         get_msgs_by_scope(scope: Vec<u8>) => Vec<BridgeMessage>;
    }
}

fn write_msg<D: BridgeMessageDb>(
    context: &Context<D>,
    id: u128,
    msg: BridgeMessage,
) -> DbResult<()> {
    context.db.write_msg(id, msg)
}

fn delete_msgs_before_timestamp<D: BridgeMessageDb>(
    context: &Context<D>,
    msg_ids: u128,
) -> DbResult<()> {
    context.db.delete_msgs_before_timestamp(msg_ids)
}

fn get_msgs_by_scope<D: BridgeMessageDb>(
    context: &Context<D>,
    scope: Vec<u8>,
) -> DbResult<Vec<BridgeMessage>> {
    context.db.get_msgs_by_scope(&scope)
}
