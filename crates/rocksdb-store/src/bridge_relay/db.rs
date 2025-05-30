#![allow(unused)]

use std::sync::Arc;

use rockbound::{
    utils::get_last, OptimisticTransactionDB as DB, SchemaBatch, SchemaDBOperationsExt,
    TransactionRetry,
};
use strata_db::{interfaces::bridge_relay::BridgeMessageDb, DbError, DbResult};
use strata_primitives::relay::types::{BridgeMessage, Scope};

use super::schemas::{BridgeMsgIdSchema, ScopeMsgIdSchema};
use crate::DbOpsConfig;

#[derive(Debug)]
pub struct BridgeMsgDb {
    db: Arc<DB>,
    ops: DbOpsConfig,
}

impl BridgeMsgDb {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(db: Arc<DB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }

    fn get_msg_ids_before_timestamp(&self, msg_id: u128) -> DbResult<Vec<u128>> {
        // reverse and then place a iterator here
        let mut iterator = self.db.iter::<BridgeMsgIdSchema>()?;
        iterator.seek_to_first();

        let mut ids = Vec::new();
        for res in iterator {
            let (timestamp, _) = res?.into_tuple();
            if timestamp <= msg_id {
                ids.push(timestamp);
            }
        }

        Ok(ids)
    }
}

impl BridgeMessageDb for BridgeMsgDb {
    fn write_msg(&self, id: u128, msg: BridgeMessage) -> strata_db::DbResult<()> {
        let mut id = id;

        self.db
            .with_optimistic_txn(TransactionRetry::Count(self.ops.retry_count), |txn| {
                while self.db.get::<BridgeMsgIdSchema>(&id)?.is_some() {
                    id += 1;
                }

                txn.put::<BridgeMsgIdSchema>(&id, &msg);

                let scope = msg.scope().to_owned();

                if let Some(scopes) = txn.get::<ScopeMsgIdSchema>(&scope)? {
                    let mut new_scopes = Vec::new();
                    new_scopes.extend(&scopes);
                    new_scopes.push(id);

                    txn.put::<ScopeMsgIdSchema>(&scope, &new_scopes)?;
                } else {
                    txn.put::<ScopeMsgIdSchema>(&scope, &vec![id])?;
                }

                Ok::<(), DbError>(())
            })
            .map_err(|e: rockbound::TransactionError<_>| DbError::TransactionError(e.to_string()))
    }

    fn delete_msgs_before_timestamp(&self, msg_id: u128) -> DbResult<()> {
        let ids = self.get_msg_ids_before_timestamp(msg_id)?;

        let mut batch = SchemaBatch::new();
        for id in ids {
            batch.delete::<BridgeMsgIdSchema>(&id)?;
        }

        self.db.write_schemas(batch)?;
        Ok(())
    }

    fn get_msgs_by_scope(&self, scope: &[u8]) -> DbResult<Vec<BridgeMessage>> {
        // Regular loop for filtering and mapping
        let Some(msg_ids) = self.db.get::<ScopeMsgIdSchema>(&scope.to_owned())? else {
            return Ok(Vec::new());
        };

        let mut msgs = Vec::new();

        // Iterating over filtered message IDs to fetch messages
        for id in msg_ids {
            let Some(message) = self.db.get::<BridgeMsgIdSchema>(&id)? else {
                continue;
            };

            msgs.push(message);
        }

        Ok(msgs)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use strata_primitives::relay::types::BridgeMessage;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> BridgeMsgDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        BridgeMsgDb::new(db, db_ops)
    }

    fn make_bridge_msg() -> (u128, BridgeMessage) {
        let mut arb = ArbitraryGenerator::new();

        let msg: BridgeMessage = arb.generate();

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros();

        (timestamp, msg)
    }

    #[test]
    fn test_write_msgs() {
        let br_db = setup_db();
        let (timestamp, msg) = make_bridge_msg();

        let result = br_db.write_msg(timestamp, msg);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_msg_ids_before_timestamp() {
        let br_db = setup_db();
        let (timestamp1, msg1) = make_bridge_msg();
        let (timestamp2, _) = make_bridge_msg();
        let (timestamp3, msg2) = make_bridge_msg();

        // Write messages to the database
        br_db.write_msg(timestamp1, msg1).unwrap();
        br_db.write_msg(timestamp3, msg2).unwrap();

        // Retrieve message IDs before the second timestamp
        let result = br_db.get_msg_ids_before_timestamp(timestamp2);
        assert!(result.is_ok());

        let ids = result.unwrap();
        assert!(ids.contains(&timestamp1));
    }

    #[test]
    fn test_delete_msgs_before_timestamp() {
        let br_db = setup_db();
        let (timestamp1, msg1) = make_bridge_msg();
        let (timestamp2, msg2) = make_bridge_msg();

        // Write messages to the database
        br_db.write_msg(timestamp1, msg1).unwrap();
        br_db.write_msg(timestamp2, msg2).unwrap();
        // Delete messages before the second timestamp
        let result = br_db.delete_msgs_before_timestamp(timestamp2);
        assert!(result.is_ok());

        // Check if only the second message remains
        let ids = br_db.get_msg_ids_before_timestamp(u128::MAX).unwrap();
        assert!(!ids.contains(&timestamp1));
    }

    #[test]
    fn test_get_msgs_by_scope() {
        let br_db = setup_db();
        let (timestamp1, mut msg1) = make_bridge_msg();
        let (timestamp2, mut msg2) = make_bridge_msg();

        // Write messages to the database
        br_db.write_msg(timestamp1, msg1.clone()).unwrap();
        br_db.write_msg(timestamp2, msg2.clone()).unwrap();

        // Retrieve messages by scope
        let result = br_db.get_msgs_by_scope(msg1.scope());
        assert!(result.is_ok());

        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_no_messages_for_nonexistent_scope() {
        let br_db = setup_db();
        let (timestamp, msg) = make_bridge_msg();
        let scope = msg.scope().to_vec();

        // Write message to the database
        br_db
            .write_msg(timestamp, msg)
            .expect("test: insert bridge msg");

        // Try to retrieve messages with a different scope
        let result = br_db
            .get_msgs_by_scope(&[42])
            .expect("test: fetch bridge msg");
        assert!(result.is_empty());

        // Try to retrieve messages with a different scope
        let result = br_db
            .get_msgs_by_scope(&scope)
            .expect("test: fetch bridge msg");

        // Should not be empty since we're using the scope of the message we put in.
        assert!(!result.is_empty());
    }
}
