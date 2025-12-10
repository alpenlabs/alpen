use std::{
    future::Future,
    time::{Duration, UNIX_EPOCH},
};

use tokio::time::sleep;

pub trait Clock: Sized {
    /// current time in milliseconds since UNIX_EPOCH
    fn current_timestamp(&self) -> u64;
    /// sleep for specified time interval.
    fn sleep_ms(&self, ms: u64) -> impl Future<Output = ()>;
    /// sleep until unix timestamp
    fn sleep_until(&self, timestamp_ms: u64) -> impl Future<Output = ()>;
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn current_timestamp(&self) -> u64 {
        UNIX_EPOCH
            .elapsed()
            .expect("system clock should be available")
            .as_millis() as u64
    }

    fn sleep_ms(&self, ms: u64) -> impl Future<Output = ()> {
        sleep(Duration::from_millis(ms))
    }

    fn sleep_until(&self, timestamp_ms: u64) -> impl Future<Output = ()> {
        let now = self.current_timestamp();
        sleep(Duration::from_millis(timestamp_ms.saturating_sub(now)))
    }
}
