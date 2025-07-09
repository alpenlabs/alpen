use std::time::{SystemTime, UNIX_EPOCH};

use strata_db::interfaces::bridge_relay::BridgeMessageDb;
use strata_primitives::relay::types::BridgeMessage;
use strata_test_utils::ArbitraryGenerator;

pub fn test_write_msgs<T: BridgeMessageDb>(db: &T) {
    let (timestamp, msg) = make_bridge_msg();

    let result = db.write_msg(timestamp, msg);
    assert!(result.is_ok());
}

pub fn test_get_msg_ids_before_timestamp<T: BridgeMessageDb>(db: &T) {
    let (timestamp1, msg1) = make_bridge_msg();
    let (_timestamp2, _) = make_bridge_msg();
    let (timestamp3, msg2) = make_bridge_msg();

    // Write messages to the database
    db.write_msg(timestamp1, msg1).unwrap();
    db.write_msg(timestamp3, msg2).unwrap();

    // Retrieve message IDs before the second timestamp
    let result = db.get_msgs_by_scope(b"dummy_scope");
    assert!(result.is_ok());
}

pub fn test_delete_msgs_before_timestamp<T: BridgeMessageDb>(db: &T) {
    let (timestamp1, msg1) = make_bridge_msg();
    let (timestamp2, msg2) = make_bridge_msg();

    // Write messages to the database
    db.write_msg(timestamp1, msg1).unwrap();
    db.write_msg(timestamp2, msg2).unwrap();
    // Delete messages before the second timestamp
    let result = db.delete_msgs_before_timestamp(timestamp2);
    assert!(result.is_ok());
}

pub fn test_get_msgs_by_scope<T: BridgeMessageDb>(db: &T) {
    let (timestamp1, msg1) = make_bridge_msg();
    let (timestamp2, msg2) = make_bridge_msg();

    // Write messages to the database
    db.write_msg(timestamp1, msg1.clone()).unwrap();
    db.write_msg(timestamp2, msg2.clone()).unwrap();

    // Retrieve messages by scope
    let result = db.get_msgs_by_scope(msg1.scope());
    assert!(result.is_ok());

    assert!(!result.unwrap().is_empty());
}

pub fn test_no_messages_for_nonexistent_scope<T: BridgeMessageDb>(db: &T) {
    let (timestamp, msg) = make_bridge_msg();
    let scope = msg.scope().to_vec();

    // Write message to the database
    db.write_msg(timestamp, msg)
        .expect("test: insert bridge msg");

    // Try to retrieve messages with a different scope
    let result = db
        .get_msgs_by_scope(&[42])
        .expect("test: fetch bridge msg");
    assert!(result.is_empty());

    // Try to retrieve messages with a different scope
    let result = db
        .get_msgs_by_scope(&scope)
        .expect("test: fetch bridge msg");

    // Should not be empty since we're using the scope of the message we put in.
    assert!(!result.is_empty());
}

// Helper function to make bridge messages
fn make_bridge_msg() -> (u128, BridgeMessage) {
    let mut arb = ArbitraryGenerator::new();

    let msg: BridgeMessage = arb.generate();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();

    (timestamp, msg)
}