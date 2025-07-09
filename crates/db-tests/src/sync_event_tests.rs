use std::time::{SystemTime, UNIX_EPOCH};

use strata_db::traits::SyncEventDatabase;
use strata_db::errors::DbError;
use strata_state::sync_event::SyncEvent;
use strata_test_utils::ArbitraryGenerator;

pub fn test_get_sync_event<T: SyncEventDatabase>(db: &T) {
    let ev1 = db.get_sync_event(1).unwrap();
    assert!(ev1.is_none());

    let ev = insert_event(db);

    let ev1 = db.get_sync_event(1).unwrap();
    assert!(ev1.is_some());

    assert_eq!(ev1.unwrap(), ev);
}

pub fn test_get_last_idx_1<T: SyncEventDatabase>(db: &T) {
    let idx = db.get_last_idx().unwrap().unwrap_or(0);
    assert_eq!(idx, 0);

    let n = 5;
    for i in 1..=n {
        let _ = insert_event(db);
        let idx = db.get_last_idx().unwrap().unwrap_or(0);
        assert_eq!(idx, i);
    }
}

pub fn test_get_timestamp<T: SyncEventDatabase>(db: &T) {
    let mut timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let n = 5;
    for i in 1..=n {
        let _ = insert_event(db);
        let ts = db.get_event_timestamp(i).unwrap().unwrap();
        assert!(ts >= timestamp);
        timestamp = ts;
    }
}

pub fn test_clear_sync_event<T: SyncEventDatabase>(db: &T) {
    let n = 5;
    for _ in 1..=n {
        let _ = insert_event(db);
    }

    // Delete events 2..4
    let res = db.clear_sync_event_range(2, 4);
    assert!(res.is_ok());

    let ev1 = db.get_sync_event(1).unwrap();
    let ev2 = db.get_sync_event(2).unwrap();
    let ev3 = db.get_sync_event(3).unwrap();
    let ev4 = db.get_sync_event(4).unwrap();
    let ev5 = db.get_sync_event(5).unwrap();

    assert!(ev1.is_some());
    assert!(ev2.is_none());
    assert!(ev3.is_none());
    assert!(ev4.is_some());
    assert!(ev5.is_some());
}

pub fn test_clear_sync_event_2<T: SyncEventDatabase>(db: &T) {
    let n = 5;
    for _ in 1..=n {
        let _ = insert_event(db);
    }
    let res = db.clear_sync_event_range(6, 7);
    assert!(res.is_err_and(|x| matches!(x, DbError::Other(ref msg) if msg == "end_idx must be less than or equal to last_key")));
}

pub fn test_get_last_idx_2<T: SyncEventDatabase>(db: &T) {
    let n = 5;
    for _ in 1..=n {
        let _ = insert_event(db);
    }
    let res = db.clear_sync_event_range(2, 3);
    assert!(res.is_ok());

    let new_idx = db.get_last_idx().unwrap().unwrap();
    assert_eq!(new_idx, 5);
}

// Helper function to insert events
fn insert_event<T: SyncEventDatabase>(db: &T) -> SyncEvent {
    let ev: SyncEvent = ArbitraryGenerator::new().generate();
    let res = db.write_sync_event(ev.clone());
    assert!(res.is_ok());
    ev
}