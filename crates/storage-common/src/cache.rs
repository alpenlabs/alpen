//! Generic cache utility for what we're inserting into the database.

use std::{hash::Hash, num::NonZeroUsize, sync::Arc};

use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::*;

use crate::exec::{GenericRecv, OpsError};

/// Entry for something we can put into the cache without actually knowing what it is, and so we can
/// keep the reservation to it.
type CacheSlot<T, E> = Arc<RwLock<SlotState<T, E>>>;

/// Describes a cache entry that may be occupied, reserved for pending database read, or returned an
/// error from a database read.
#[derive(Debug)]
pub enum SlotState<T, E> {
    /// Authentic database entry.
    Ready(T),

    /// A database fetch is happening in the background and it will be updated.
    Pending(broadcast::Receiver<Result<T, E>>),

    /// An error happened fetching from the database.
    Error(E),
}

/// Wrapper around a LRU cache that handles cache reservations and asynchronously waiting for
/// database operations in the background without keeping a global lock on the cache.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct CacheTable<K, V, E> {
    cache: Mutex<lru::LruCache<K, CacheSlot<V, E>>>,
}

impl<K: Clone + Eq + Hash, V: Clone, E: From<OpsError> + std::error::Error + Clone>
    CacheTable<K, V, E>
{
    /// Creates a new cache with some maximum capacity.
    ///
    /// This measures entries by *count* not their (serialized?) size, so ideally entries should
    /// consume similar amounts of memory to helps us best reason about real cache capacity.
    pub fn new(size: NonZeroUsize) -> Self {
        Self {
            cache: Mutex::new(lru::LruCache::new(size)),
        }
    }

    /// Gets the number of elements in the cache.
    // TODO replace this with an atomic we update after every op
    #[allow(dead_code, clippy::allow_attributes, reason = "used for debugging")]
    pub async fn get_len_async(&self) -> usize {
        let cache = self.cache.lock().await;
        cache.len()
    }

    /// Gets the number of elements in the cache.
    // TODO replace this with an atomic we update after every op
    #[allow(dead_code, clippy::allow_attributes, reason = "used for debugging")]
    pub fn get_len_blocking(&self) -> usize {
        let cache = self.cache.blocking_lock();
        cache.len()
    }

    /// Removes the entry for a particular cache entry.
    pub async fn purge_async(&self, k: &K) {
        let mut cache = self.cache.lock().await;
        cache.pop(k);
    }

    /// Removes the entry for a particular cache entry.
    pub fn purge_blocking(&self, k: &K) {
        let mut cache = self.cache.blocking_lock();
        cache.pop(k);
    }

    /// Removes all entries for which the predicate fails.  Returns the number
    /// of entries removed.
    ///
    /// This unfortunately has to clone as many keys from the cache as pass the
    /// predicate, which means it's capped at the maximum size of the cache, so
    /// that's not *so* bad.
    ///
    /// This might remove slots that are in the process of being filled.  Those
    /// operations will complete, but we won't retain those values.
    pub async fn purge_if_async(&self, mut pred: impl FnMut(&K) -> bool) -> usize {
        let mut cache = self.cache.lock().await;
        let keys_to_remove = cache
            .iter()
            .map(|(k, _v)| k)
            .filter(|k| pred(k)) // why can't I just pass pred?
            .cloned()
            .collect::<Vec<_>>();
        keys_to_remove.iter().for_each(|k| {
            cache.pop(k);
        });
        keys_to_remove.len()
    }
    /// Removes all entries for which the predicate fails.  Returns the number
    /// of entries removed.
    ///
    /// This unfortunately has to clone as many keys from the cache as pass the
    /// predicate, which means it's capped at the maximum size of the cache, so
    /// that's not *so* bad.
    ///
    /// This might remove slots that are in the process of being filled.  Those
    /// operations will complete, but we won't retain those values.
    pub fn purge_if_blocking(&self, mut pred: impl FnMut(&K) -> bool) -> usize {
        let mut cache = self.cache.blocking_lock();
        let keys_to_remove = cache
            .iter()
            .map(|(k, _v)| k)
            .filter(|k| pred(k)) // why can't I just pass pred?
            .cloned()
            .collect::<Vec<_>>();
        keys_to_remove.iter().for_each(|k| {
            cache.pop(k);
        });
        keys_to_remove.len()
    }

    /// Removes all entries from the cache.  Returns the number of entries
    /// removed.
    ///
    /// This might remove slots that are in the process of being filled.  Those
    /// operations will complete, but we won't retain those values.
    pub fn blocking_clear(&self) -> usize {
        let mut cache = self.cache.blocking_lock();
        let len = cache.len();
        cache.clear();
        len
    }

    /// Inserts an entry into the table, dropping the previous value.
    pub async fn insert_async(&self, k: K, v: V) {
        let slot = Arc::new(RwLock::new(SlotState::Ready(v)));
        self.cache.lock().await.put(k, slot);
    }

    /// Inserts an entry into the table, dropping the previous value.
    pub fn insert_blocking(&self, k: K, v: V) {
        let slot = Arc::new(RwLock::new(SlotState::Ready(v)));
        self.cache.blocking_lock().put(k, slot);
    }

    /// Returns a clone of an entry from the cache or possibly invoking some function returning a
    /// `oneshot` channel that will return the value from the underlying database.
    ///
    /// This is meant to be used with the `_chan` functions generated by the db ops macro in the
    /// `exec` module.
    pub async fn get_or_fetch(
        &self,
        k: &K,
        fetch_fn: impl Fn() -> GenericRecv<V, E>,
    ) -> Result<V, E> {
        // See below comment about control flow.
        let (slot, complete_tx) = {
            let mut cache_guard = self.cache.lock().await;
            trace!("acquired cache lock");
            if let Some(entry_guard) = cache_guard.get(k).cloned() {
                drop(cache_guard);

                // Get the state and extract what we need before dropping the lock
                let mut receiver = {
                    let entry_guard = entry_guard.read().await;
                    trace!("acquired slot read lock");
                    match &*entry_guard {
                        SlotState::Ready(v) => return Ok(v.clone()),
                        SlotState::Pending(ch) => ch.resubscribe(),
                        SlotState::Error(e) => return Err(e.clone()),
                    }
                }; // read lock dropped here

                // Now wait on the channel without holding any locks
                trace!("waiting for database fetch to complete");
                match receiver.recv().await {
                    Ok(result) => return result,
                    Err(_e) => return Err(OpsError::WorkerFailedStrangely.into()),
                }
            }

            // Create a new cache slot and insert and lock it.
            let (complete_tx, complete_rx) = broadcast::channel(1);
            let slot = Arc::new(RwLock::new(SlotState::Pending(complete_rx)));
            cache_guard.push(k.clone(), slot.clone());

            (slot, complete_tx)
        };

        // Make the fetch.
        let fetch_res = fetch_fn().await;

        // Some error logging before we try to acquire locks.
        if fetch_res.is_err() {
            error!("database fetch aborted");
        }

        if let Ok(Err(e)) = fetch_res.as_ref() {
            error!(%e, "failed to make database fetch");
        }

        // And then re-acquire the lock on the slot before handling the result.
        let mut slot_guard = slot.write().await;
        trace!("acquired slot write lock");
        match fetch_res {
            Ok(Ok(v)) => {
                // Store value in slot and broadcast
                *slot_guard = SlotState::Ready(v.clone());
                let _ = complete_tx.send(Ok(v.clone()));
                Ok(v)
            }

            Ok(Err(err)) => {
                // Store error in slot and broadcast
                *slot_guard = SlotState::Error(err.clone());
                let _ = complete_tx.send(Err(err.clone()));
                drop(slot_guard);
                let mut cache_guard = self.cache.lock().await;
                trace!("re-acquired cache lock");
                safely_remove_cache_slot(&mut cache_guard, k, &slot);

                Err(err)
            }

            Err(_) => {
                // Send the error to waiting tasks and update slot
                let error = E::from(OpsError::WorkerFailedStrangely);
                *slot_guard = SlotState::Error(error.clone());
                let _ = complete_tx.send(Err(error.clone()));
                drop(slot_guard);
                let mut cache_guard = self.cache.lock().await;
                trace!("re-acquired cache lock");
                safely_remove_cache_slot(&mut cache_guard, k, &slot);

                Err(error.into())
            }
        }
    }

    /// Returns a clone of an entry from the cache or invokes some function to load it from
    /// the underlying database.
    pub fn get_or_fetch_blocking(
        &self,
        k: &K,
        fetch_fn: impl Fn() -> Result<V, E>,
    ) -> Result<V, E> {
        // The flow control here is kinda weird, I don't like it.  The key here is that we want to
        // ensure the lock on the whole cache is as short-lived as possible while we check to see if
        // the entry we're looking for is there.  If it's not, then we want to insert a reservation
        // that we hold a lock to and then release the cache-level lock.
        let (slot, complete_tx) = {
            let mut cache_guard = self.cache.blocking_lock();
            if let Some(entry_guard) = cache_guard.get(k).cloned() {
                drop(cache_guard);

                // Get the state and extract what we need before dropping the lock
                let mut receiver = {
                    let entry_guard = entry_guard.blocking_read();
                    trace!("acquired slot read lock");
                    match &*entry_guard {
                        SlotState::Ready(v) => return Ok(v.clone()),
                        SlotState::Pending(ch) => ch.resubscribe(),
                        SlotState::Error(e) => return Err(e.clone()),
                    }
                }; // read lock dropped here

                // Now wait on the channel without holding any locks
                trace!("waiting for database fetch to complete");
                match receiver.blocking_recv() {
                    Ok(result) => return result,
                    Err(_e) => return Err(OpsError::WorkerFailedStrangely.into()),
                }
            }

            // Create a new cache slot and insert and lock it.
            let (complete_tx, complete_rx) = broadcast::channel(1);
            let slot = Arc::new(RwLock::new(SlotState::Pending(complete_rx)));
            cache_guard.push(k.clone(), slot.clone());

            (slot, complete_tx)
        };

        // Load the entry and insert it into the slot we've already reserved.
        let fetch_res = fetch_fn();

        // Some error logging before we try to acquire locks.
        if let Err(e) = fetch_res.as_ref() {
            warn!(%e, "failed to make database fetch");
        }

        // And then re-acquire the lock on the slot before handling the result.
        let mut slot_guard = slot.blocking_write();
        trace!("re-acquired slot lock");
        match fetch_res {
            Ok(v) => {
                // Store value in slot and broadcast
                *slot_guard = SlotState::Ready(v.clone());
                let _ = complete_tx.send(Ok(v.clone()));
                Ok(v)
            }

            Err(e) => {
                // Store error in slot and broadcast
                *slot_guard = SlotState::Error(e.clone());
                let _ = complete_tx.send(Err(e.clone()));
                drop(slot_guard);
                let mut cache_guard = self.cache.blocking_lock();
                trace!("re-acquired cache lock");
                safely_remove_cache_slot(&mut cache_guard, k, &slot);

                Err(e)
            }
        }
    }
}

/// Convenience function to safely remove a cache slot key from the cache, iff
/// it matches an expected value. Returns if it did the removal.
fn safely_remove_cache_slot<K: Eq + Hash, V, E>(
    cache: &mut lru::LruCache<K, CacheSlot<V, E>>,
    key: &K,
    slot: &CacheSlot<V, E>,
) -> bool {
    let is_eq = Arc::ptr_eq(cache.peek(key).unwrap(), slot);
    if is_eq {
        cache.pop(key);
    }
    is_eq
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use thiserror::Error;

    use crate::exec::OpsError;

    #[derive(Debug, Clone, Error)]
    enum DbError {
        #[error("resource busy")]
        Busy,
        #[error("entry with idx does not exist")]
        NonExistentEntry,
        #[error("worked task exited strangely")]
        WorkerFailedStrangely,
    }

    impl From<OpsError> for DbError {
        fn from(value: OpsError) -> Self {
            match value {
                OpsError::WorkerFailedStrangely => DbError::WorkerFailedStrangely,
            }
        }
    }

    type CacheTable<K, V> = super::CacheTable<K, V, DbError>;

    #[tokio::test]
    async fn test_basic_async() {
        let cache = CacheTable::<u64, u64>::new(3.try_into().unwrap());

        let res = cache
            .get_or_fetch(&42, || {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tx.send(Ok(10)).expect("test: send init value");
                rx
            })
            .await
            .expect("test: cache gof");
        assert_eq!(res, 10);

        let res = cache
            .get_or_fetch(&42, || {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tx.send(Err(DbError::Busy)).expect("test: send init value");
                rx
            })
            .await
            .expect("test: load gof");
        assert_eq!(res, 10);

        cache.insert_async(42, 12).await;
        let res = cache
            .get_or_fetch(&42, || {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tx.send(Err(DbError::Busy)).expect("test: send init value");
                rx
            })
            .await
            .expect("test: load gof");
        assert_eq!(res, 12);

        let len = cache.get_len_async().await;
        assert_eq!(len, 1);
        cache.purge_async(&42).await;
        let len = cache.get_len_async().await;
        assert_eq!(len, 0);
    }

    #[test]
    fn test_basic_blocking() {
        let cache = CacheTable::<u64, u64>::new(3.try_into().unwrap());

        let res = cache
            .get_or_fetch_blocking(&42, || Ok(10))
            .expect("test: cache gof");
        assert_eq!(res, 10);

        let res = cache
            .get_or_fetch_blocking(&42, || Err(DbError::Busy))
            .expect("test: load gof");
        assert_eq!(res, 10);

        cache.insert_blocking(42, 12);
        let res = cache
            .get_or_fetch_blocking(&42, || Err(DbError::Busy))
            .expect("test: load gof");
        assert_eq!(res, 12);

        let len = cache.get_len_blocking();
        assert_eq!(len, 1);
        cache.purge_blocking(&42);
        let len = cache.get_len_blocking();
        assert_eq!(len, 0);
    }

    #[tokio::test]
    async fn test_deadlock_scenario() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(3.try_into().unwrap()));
        let key = 42u64;

        // Task 1: Creates a pending slot and starts a slow fetch
        let cache1 = cache.clone();
        let task1 = tokio::spawn(async move {
            cache1
                .get_or_fetch(&key, || {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    // Spawn a task that will complete after a delay
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        tx.send(Ok(100)).unwrap();
                    });
                    rx
                })
                .await
        });

        // Small delay to ensure task1 creates the pending slot first
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Task 2: Tries to read from the same key (will find pending slot)
        let cache2 = cache.clone();
        let task2 = tokio::spawn(async move {
            cache2
                .get_or_fetch(&key, || {
                    // This should never be called since the slot already exists
                    panic!("This fetch function should not be called");
                })
                .await
        });

        // Use timeout to detect deadlock - in deadlock scenario this will timeout
        let timeout_result = tokio::time::timeout(Duration::from_secs(2), async {
            let (result1, result2) = tokio::join!(task1, task2);
            match (result1, result2) {
                (Ok(val1), Ok(val2)) => Ok((val1, val2)),
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        })
        .await;

        match timeout_result {
            Ok(Ok((Ok(val1), Ok(val2)))) => {
                // No deadlock - both tasks completed successfully
                assert_eq!(val1, 100);
                assert_eq!(val2, 100);
            }
            Ok(Ok((Err(e), _))) | Ok(Ok((_, Err(e)))) => {
                panic!("Database error: {e:?}");
            }
            Ok(Err(e)) => {
                panic!("Task join error: {e:?}");
            }
            Err(_) => {
                // Timeout occurred - indicates deadlock
                panic!("DEADLOCK DETECTED: Tasks timed out after 2 seconds");
            }
        }
    }

    #[test]
    fn test_deadlock_scenario_blocking() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(3.try_into().unwrap()));
        let key = 42u64;

        // Task 1: Creates a pending slot and starts a slow fetch
        let cache1 = cache.clone();
        let handle1 = std::thread::spawn(move || {
            cache1.get_or_fetch_blocking(&key, || {
                // Simulate slow database operation
                std::thread::sleep(Duration::from_millis(100));
                Ok(100)
            })
        });

        // Small delay to ensure task1 creates the pending slot first
        std::thread::sleep(Duration::from_millis(10));

        // Task 2: Tries to read from the same key (will find pending slot)
        let cache2 = cache.clone();
        let handle2 = std::thread::spawn(move || {
            cache2.get_or_fetch_blocking(&key, || {
                // This should never be called since the slot already exists
                panic!("This fetch function should not be called");
            })
        });

        // Use timeout to detect deadlock - in deadlock scenario this will timeout
        let start_time = std::time::Instant::now();
        let timeout_duration = Duration::from_secs(2);

        // Wait for both threads with timeout
        let result1 = wait_for_thread_with_timeout(handle1, timeout_duration);
        let result2 = wait_for_thread_with_timeout(handle2, timeout_duration);

        let elapsed = start_time.elapsed();

        match (result1, result2) {
            (Some(Ok(Ok(val1))), Some(Ok(Ok(val2)))) => {
                // No deadlock - both tasks completed successfully
                assert_eq!(val1, 100);
                assert_eq!(val2, 100);
            }
            (Some(Ok(Err(e))), _) | (_, Some(Ok(Err(e)))) => {
                panic!("Database error: {e:?}");
            }
            (Some(Err(e)), _) | (_, Some(Err(e))) => {
                panic!("Thread panic: {e:?}");
            }
            _ => {
                // Timeout occurred - indicates deadlock
                if elapsed >= timeout_duration {
                    panic!("DEADLOCK DETECTED: Tasks timed out after {elapsed:?}");
                } else {
                    panic!("Thread join failed unexpectedly");
                }
            }
        }
    }

    fn wait_for_thread_with_timeout<T>(
        handle: std::thread::JoinHandle<T>,
        timeout: Duration,
    ) -> Option<std::thread::Result<T>> {
        let start = std::time::Instant::now();
        loop {
            if handle.is_finished() {
                return Some(handle.join()); // Thread is finished, try to join
            }
            if start.elapsed() >= timeout {
                return None; // Timeout reached
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[tokio::test]
    async fn test_concurrent_error_propagation() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(3.try_into().unwrap()));
        let key = 42u64;

        // Task 1: Creates a pending slot and will return a specific DbError
        let cache1 = cache.clone();
        let task1 = tokio::spawn(async move {
            cache1
                .get_or_fetch(&key, || {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    // Spawn a task that will return a specific error after delay
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        tx.send(Err(DbError::NonExistentEntry)).unwrap(); // Specific error
                    });
                    rx
                })
                .await
        });

        // Small delay to ensure task1 creates the pending slot first
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Task 2: Tries to read from the same key (will find pending slot)
        let cache2 = cache.clone();
        let task2 = tokio::spawn(async move {
            cache2
                .get_or_fetch(&key, || {
                    // This should never be called since the slot already exists
                    panic!("This fetch function should not be called");
                })
                .await
        });

        // Wait for both tasks to complete
        let (result1, result2) = tokio::join!(task1, task2);

        let error1 = result1
            .expect("Task 1 should not panic")
            .expect_err("Task 1 should return error");
        let error2 = result2
            .expect("Task 2 should not panic")
            .expect_err("Task 2 should return error");

        // Both tasks should get the same specific DbError::NonExistentEntry
        assert!(
            matches!(error1, DbError::NonExistentEntry),
            "Task 1 should get DbError::NonExistentEntry, got: {error1:?}",
        );
        assert!(
            matches!(error2, DbError::NonExistentEntry),
            "Task 2 should also get DbError::NonExistentEntry, got: {error1:?}",
        );
    }

    #[test]
    fn test_concurrent_error_propagation_blocking() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(3.try_into().unwrap()));
        let key = 42u64;

        // Task 1: Creates a pending slot and will return a specific DbError
        let cache1 = cache.clone();
        let handle1 = std::thread::spawn(move || {
            cache1.get_or_fetch_blocking(&key, || {
                // Simulate slow database operation that returns specific error
                std::thread::sleep(Duration::from_millis(50));
                Err(DbError::NonExistentEntry) // Specific error
            })
        });

        // Small delay to ensure task1 creates the pending slot first
        std::thread::sleep(Duration::from_millis(10));

        // Task 2: Tries to read from the same key (will find pending slot)
        let cache2 = cache.clone();
        let handle2 = std::thread::spawn(move || {
            cache2.get_or_fetch_blocking(&key, || {
                // This should never be called since the slot already exists
                panic!("This fetch function should not be called");
            })
        });

        // Wait for both threads to complete
        let result1 = handle1.join().expect("Thread 1 should not panic");
        let result2 = handle2.join().expect("Thread 2 should not panic");

        let error1 = result1.expect_err("Task 1 should return error");
        let error2 = result2.expect_err("Task 2 should return error");

        // Both tasks should get the same specific DbError::NonExistentEntry
        assert!(
            matches!(error1, DbError::NonExistentEntry),
            "Task 1 should get DbError::NonExistentEntry, got: {error1:?}"
        );
        assert!(
            matches!(error2, DbError::NonExistentEntry),
            "Task 2 should also get DbError::NonExistentEntry, got: {error1:?}"
        );
    }
}
