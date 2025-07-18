use std::time::{SystemTime, UNIX_EPOCH};

use borsh::{BorshDeserialize, BorshSerialize};
use strata_state::sync_event::SyncEvent;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SyncEventWithTimestamp {
    event: SyncEvent,
    timestamp: u64,
}

impl SyncEventWithTimestamp {
    pub fn new(event: SyncEvent) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        SyncEventWithTimestamp { event, timestamp }
    }

    pub fn timestamp(self) -> u64 {
        self.timestamp
    }

    pub fn event(self) -> SyncEvent {
        self.event
    }
}

// Sync Event Schema and corresponding codecs implementation
define_table_with_seek_key_codec!(
    /// A table to store Sync Events. Maps event index to event
    (SyncEventSchema) u64 => SyncEventWithTimestamp
);
