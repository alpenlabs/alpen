//! Generic cache utility for what we're inserting into the database.

use std::{hash::Hash, num::NonZeroUsize, sync::Arc};

use parking_lot::Mutex;
use strata_db::{DbError, DbResult};
use tokio::sync::{broadcast, broadcast::error::SendError, RwLock};
use tracing::*;

use crate::exec::DbRecv;

/// Entry for something we can put into the cache without actually knowing what it is, and so we can
/// keep the reservation to it.
type CacheSlot<T> = Arc<RwLock<SlotState<T>>>;

/// Describes a cache entry that may be occupied, reserved for pending database read, or returned an
/// error from a database read.
#[derive(Debug)]
pub(crate) enum SlotState<T> {
    /// Authentic database entry.
    Ready(T),

    /// A database fetch is happening in the background and it will be updated.
    Pending(broadcast::Receiver<T>),

    /// An unspecified error happened fetching from the database.
    Error,
}

impl<T: Clone> SlotState<T> {
    /// Tries to read a value from the slot, asynchronously.
    pub(crate) async fn get_async(&self) -> DbResult<T> {
        match self {
            Self::Ready(v) => Ok(v.clone()),
            Self::Pending(ch) => {
                // When we see this log get triggered and but feels like the corresponding fetch is
                // hanging for this read then it means that this code wasn't implemented
                // correctly.
                // TODO figure out how to test this
                trace!("waiting for database fetch to complete");
                match ch.resubscribe().recv().await {
                    Ok(v) => Ok(v),
                    Err(_e) => Err(DbError::WorkerFailedStrangely),
                }
            }
            Self::Error => Err(DbError::CacheLoadFail),
        }
    }

    /// Tries to read a value from the slot, blockingly.
    pub(crate) fn get_blocking(&self) -> DbResult<T> {
        match self {
            Self::Ready(v) => Ok(v.clone()),
            Self::Pending(ch) => {
                // When we see this log get triggered and but feels like the corresponding fetch is
                // hanging for this read then it means that this code wasn't implemented
                // correctly.
                // TODO figure out how to test this
                trace!("waiting for database fetch to complete");
                match ch.resubscribe().blocking_recv() {
                    Ok(v) => Ok(v),
                    Err(_e) => Err(DbError::WorkerFailedStrangely),
                }
            }
            Self::Error => Err(DbError::CacheLoadFail),
        }
    }
}

/// Wrapper around a LRU cache that handles cache reservations and asynchronously waiting for
/// database operations in the background without keeping a global lock on the cache.
pub(crate) struct CacheTable<K, V> {
    cache: Mutex<lru::LruCache<K, CacheSlot<V>>>,
}

impl<K: Clone + Eq + Hash, V: Clone> CacheTable<K, V> {
    /// Creates a new cache with some maximum capacity.
    ///
    /// This measures entries by *count* not their (serialized?) size, so ideally entries should
    /// consume similar amounts of memory to helps us best reason about real cache capacity.
    pub(crate) fn new(size: NonZeroUsize) -> Self {
        Self {
            cache: Mutex::new(lru::LruCache::new(size)),
        }
    }

    /// Gets the number of elements in the cache.
    // TODO replace this with an atomic we update after every op
    #[allow(dead_code)]
    pub(crate) fn get_len(&self) -> usize {
        let cache = self.cache.lock();
        cache.len()
    }

    /// Removes the entry for a particular cache entry.
    pub(crate) fn purge(&self, k: &K) {
        let mut cache = self.cache.lock();
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
    pub(crate) fn purge_if(&self, mut pred: impl FnMut(&K) -> bool) -> usize {
        let mut cache = self.cache.lock();
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
    #[allow(dead_code)]
    pub(crate) fn clear(&self) -> usize {
        let mut cache = self.cache.lock();
        let len = cache.len();
        cache.clear();
        len
    }

    /// Inserts an entry into the table, dropping the previous value.
    #[allow(dead_code)]
    pub(crate) fn insert(&self, k: K, v: V) {
        let slot = Arc::new(RwLock::new(SlotState::Ready(v)));
        self.cache.lock().put(k, slot);
    }

    /// Returns a clone of an entry from the cache or possibly invoking some function returning a
    /// `oneshot` channel that will return the value from the underlying database.
    ///
    /// This is meant to be used with the `_chan` functions generated by the db ops macro in the
    /// `exec` module.
    // https://github.com/rust-lang/rust-clippy/issues/6446
    #[allow(clippy::await_holding_lock)]
    pub(crate) async fn get_or_fetch(
        &self,
        k: &K,
        fetch_fn: impl Fn() -> DbRecv<V>,
    ) -> DbResult<V> {
        // See below comment about control flow.
        let (slot, complete_tx) = {
            let mut cache_guard = self.cache.lock();
            if let Some(entry_guard) = cache_guard.get(k).cloned() {
                drop(cache_guard);
                let entry_guard = entry_guard.read().await;
                return entry_guard.get_async().await;
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
        trace!("re-acquired slot lock");
        match fetch_res {
            Ok(Ok(v)) => {
                send_completion_and_assign_slot_ready(&v, &mut slot_guard, complete_tx);
                Ok(v)
            }

            Ok(Err(e)) => {
                // Important ordering for the locks.
                let mut cache_guard = self.cache.lock();
                trace!("re-acquired cache lock");
                *slot_guard = SlotState::Error;
                safely_remove_cache_slot(&mut cache_guard, k, &slot);

                Err(e)
            }

            Err(_) => {
                // Important ordering for the locks.
                let mut cache_guard = self.cache.lock();
                trace!("re-acquired cache lock");
                *slot_guard = SlotState::Error;
                safely_remove_cache_slot(&mut cache_guard, k, &slot);

                Err(DbError::WorkerFailedStrangely)
            }
        }
    }

    /// Returns a clone of an entry from the cache or invokes some function to load it from
    /// the underlying database.
    pub(crate) fn get_or_fetch_blocking(
        &self,
        k: &K,
        fetch_fn: impl Fn() -> DbResult<V>,
    ) -> DbResult<V> {
        // The flow control here is kinda weird, I don't like it.  The key here is that we want to
        // ensure the lock on the whole cache is as short-lived as possible while we check to see if
        // the entry we're looking for is there.  If it's not, then we want to insert a reservation
        // that we hold a lock to and then release the cache-level lock.
        let (slot, complete_tx) = {
            let mut cache_guard = self.cache.lock();
            if let Some(entry_guard) = cache_guard.get(k).cloned() {
                drop(cache_guard);
                let entry_guard = entry_guard.blocking_read();
                return entry_guard.get_blocking();
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
                // Fill in the lock state and send down the complete tx.
                send_completion_and_assign_slot_ready(&v, &mut slot_guard, complete_tx);
                Ok(v)
            }

            Err(e) => {
                // Important ordering for the locks.
                let mut cache_guard = self.cache.lock();
                trace!("re-acquired cache lock");
                *slot_guard = SlotState::Error;
                safely_remove_cache_slot(&mut cache_guard, k, &slot);

                Err(e)
            }
        }
    }
}

/// Convenience function to cafefully avoid making extra clones when sending on
/// a channel and updating a cache slot.
fn send_completion_and_assign_slot_ready<T: Clone>(
    v: &T,
    slot_state: &mut SlotState<T>,
    tx: broadcast::Sender<T>,
) {
    // Try sending it on the channel first with a new clone.
    match tx.send(v.clone()) {
        Ok(waiter_cnt) => {
            trace!(%waiter_cnt, "notified cache waiters");

            // If it's consumed, then we do have to make another clone to store
            // in the lock.
            *slot_state = SlotState::Ready(v.clone());
        }

        Err(SendError(vv)) => {
            // In this (likely) case, there's no readers, so we can keep the
            // value and avoid making an additional clone.
            *slot_state = SlotState::Ready(vv);
        }
    }
}

/// Convenience function to safely remove a cache slot key from the cache, iff
/// it matches an expected value.  Returns if it did the removal.
fn safely_remove_cache_slot<K: Eq + Hash, V>(
    cache: &mut lru::LruCache<K, CacheSlot<V>>,
    key: &K,
    slot: &CacheSlot<V>,
) -> bool {
    let is_eq = Arc::ptr_eq(cache.peek(key).unwrap(), slot);
    if is_eq {
        cache.pop(key);
    }
    is_eq
}

#[cfg(test)]
mod tests {
    use strata_db::DbError;

    use super::CacheTable;

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

        cache.insert(42, 12);
        let res = cache
            .get_or_fetch(&42, || {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tx.send(Err(DbError::Busy)).expect("test: send init value");
                rx
            })
            .await
            .expect("test: load gof");
        assert_eq!(res, 12);

        let len = cache.get_len();
        assert_eq!(len, 1);
        cache.purge(&42);
        let len = cache.get_len();
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

        cache.insert(42, 12);
        let res = cache
            .get_or_fetch_blocking(&42, || Err(DbError::Busy))
            .expect("test: load gof");
        assert_eq!(res, 12);

        let len = cache.get_len();
        assert_eq!(len, 1);
        cache.purge(&42);
        let len = cache.get_len();
        assert_eq!(len, 0);
    }
}

#[cfg(test)]
mod ai_tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use tokio::{sync::Barrier, time::sleep};

    use super::*;

    // Helper to create a test cache
    fn make_cache_table() -> CacheTable<u64, String> {
        CacheTable::new(10.try_into().unwrap())
    }

    #[tokio::test]
    async fn test_concurrent_same_key_fetch() {
        let cache = Arc::new(make_cache_table());
        let fetch_count = Arc::new(AtomicUsize::new(0));

        // Spawn multiple tasks trying to fetch the same key
        let mut handles = vec![];
        for i in 0..10 {
            let cache_clone = cache.clone();
            let fetch_count_clone = fetch_count.clone();

            let handle = tokio::task::spawn_local(async move {
                let result = cache_clone
                    .get_or_fetch(&42, || {
                        let count = fetch_count_clone.fetch_add(1, Ordering::SeqCst);

                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::task::spawn_local(async move {
                            // Simulate slow database fetch
                            sleep(Duration::from_millis(100)).await;
                            tx.send(Ok(format!("value-{}", count))).unwrap();
                        });
                        rx
                    })
                    .await;

                (i, result)
            });
            handles.push(handle);
        }

        // Wait for all tasks
        let mut results = vec![];
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // All should get the same value (from the first fetch)
        let first_value = &results[0].1.as_ref().cloned().unwrap();
        for (task_id, result) in results {
            assert_eq!(
                result.as_ref().unwrap(),
                first_value,
                "Task {task_id} got different value",
            );
        }

        // Only one fetch should have occurred
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "Expected exactly one fetch"
        );
    }

    #[tokio::test]
    async fn test_concurrent_different_keys() {
        let cache = Arc::new(make_cache_table());
        let fetch_count = Arc::new(AtomicUsize::new(0));

        // Spawn tasks for different keys
        let mut handles = vec![];
        for i in 0..10 {
            let cache_clone = cache.clone();
            let fetch_count_clone = fetch_count.clone();

            let handle = tokio::task::spawn_local(async move {
                let key = i as u64;
                let result = cache_clone
                    .get_or_fetch(&key, || {
                        fetch_count_clone.fetch_add(1, Ordering::SeqCst);

                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::task::spawn_local(async move {
                            sleep(Duration::from_millis(50)).await;
                            tx.send(Ok(format!("value-{}", key))).unwrap();
                        });
                        rx
                    })
                    .await;

                (key, result)
            });
            handles.push(handle);
        }

        // Wait for all tasks
        let mut results = vec![];
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // Each should get its own value
        for (key, result) in results {
            assert_eq!(result.unwrap(), format!("value-{}", key));
        }

        // Should have 10 fetches (one per key)
        assert_eq!(fetch_count.load(Ordering::SeqCst), 10);
    }

    #[tokio::test]
    async fn test_cache_eviction_during_fetch() {
        // Small cache to force eviction
        let cache = Arc::new(CacheTable::<u64, String>::new(2.try_into().unwrap()));

        // Start a slow fetch for key 1
        let cache_clone = cache.clone();
        let slow_fetch = tokio::task::spawn_local(async move {
            cache_clone
                .get_or_fetch(&1, || {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    tokio::task::spawn_local(async move {
                        sleep(Duration::from_millis(200)).await;
                        tx.send(Ok("slow-value".to_string())).unwrap();
                    });
                    rx
                })
                .await
        });

        // Give it time to start
        sleep(Duration::from_millis(50)).await;

        // Fill cache with other entries to potentially evict key 1
        for i in 2..=5 {
            cache.insert(i, format!("fast-value-{}", i));
        }

        // The slow fetch should still complete successfully
        let result = slow_fetch.await.unwrap();
        assert_eq!(result.unwrap(), "slow-value");
    }

    #[tokio::test]
    async fn test_error_handling_concurrent() {
        let cache = Arc::new(make_cache_table());
        let barrier = Arc::new(Barrier::new(5));

        // Spawn multiple tasks that will all try to fetch the same key
        // but the fetch will fail
        let mut handles = vec![];
        for i in 0..5 {
            let cache_clone = cache.clone();
            let barrier_clone = barrier.clone();

            let handle = tokio::task::spawn_local(async move {
                // Wait for all tasks to be ready
                barrier_clone.wait().await;

                let result = cache_clone
                    .get_or_fetch(&42, || {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::task::spawn_local(async move {
                            sleep(Duration::from_millis(100)).await;
                            tx.send(Err(DbError::CacheLoadFail)).unwrap();
                        });
                        rx
                    })
                    .await;

                (i, result)
            });
            handles.push(handle);
        }

        // All should get the same error
        for handle in handles {
            let (task_id, result) = handle.await.unwrap();
            assert!(
                result.is_err(),
                "Task {} should have gotten an error",
                task_id
            );
        }

        // Cache should not contain the failed entry
        assert_eq!(cache.get_len(), 0);
    }

    #[tokio::test]
    async fn test_mixed_async_blocking() {
        let cache = Arc::new(make_cache_table());
        let fetch_count = Arc::new(AtomicUsize::new(0));

        // Mix of async and blocking operations
        let mut handles = vec![];

        // Async tasks
        for i in 0..5 {
            let cache_clone = cache.clone();
            let fetch_count_clone = fetch_count.clone();

            let handle = tokio::task::spawn_local(async move {
                cache_clone
                    .get_or_fetch(&42, || {
                        fetch_count_clone.fetch_add(1, Ordering::SeqCst);

                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::task::spawn_local(async move {
                            sleep(Duration::from_millis(100)).await;
                            tx.send(Ok(format!("async-value"))).unwrap();
                        });
                        rx
                    })
                    .await
            });
            handles.push(handle);
        }

        // Blocking tasks
        for i in 0..5 {
            let cache_clone = cache.clone();
            let fetch_count_clone = fetch_count.clone();

            let handle = tokio::task::spawn_blocking(move || {
                cache_clone.get_or_fetch_blocking(&42, || {
                    fetch_count_clone.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(100));
                    Ok("blocking-value".to_string())
                })
            });
            handles.push(handle);
        }

        // Wait for all
        let mut results = vec![];
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // All should succeed and get the same value
        let first_value = results[0].as_ref().unwrap();
        for result in &results {
            assert_eq!(result.as_ref().unwrap(), first_value);
        }

        // Should only have one fetch
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_purge_during_fetch() {
        let cache = Arc::new(make_cache_table());

        // Start a fetch
        let cache_clone = cache.clone();
        let fetch_handle = tokio::task::spawn_local(async move {
            cache_clone
                .get_or_fetch(&42, || {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    tokio::task::spawn_local(async move {
                        sleep(Duration::from_millis(200)).await;
                        tx.send(Ok("fetched-value".to_string())).unwrap();
                    });
                    rx
                })
                .await
        });

        // Let fetch start
        sleep(Duration::from_millis(50)).await;

        // Purge the key while fetch is in progress
        cache.purge(&42);

        // Fetch should still complete
        let result = fetch_handle.await.unwrap();
        assert_eq!(result.unwrap(), "fetched-value");

        // But cache should be empty (purged entry won't be retained)
        assert_eq!(cache.get_len(), 0);
    }

    #[test]
    fn test_stress_blocking() {
        use std::thread;

        let cache = Arc::new(make_cache_table());
        let fetch_count = Arc::new(AtomicUsize::new(0));

        // Spawn multiple threads
        let mut handles = vec![];
        for i in 0..10 {
            let cache_clone = cache.clone();
            let fetch_count_clone = fetch_count.clone();

            let handle = thread::spawn(move || {
                let key = (i % 3) as u64; // Use only 3 keys to create contention

                cache_clone.get_or_fetch_blocking(&key, || {
                    fetch_count_clone.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(100));
                    Ok(format!("value-{}", key))
                })
            });
            handles.push(handle);
        }

        // Wait for all threads
        let mut results = vec![];
        for handle in handles {
            results.push(handle.join().unwrap());
        }

        // Should have exactly 3 fetches (one per unique key)
        assert_eq!(fetch_count.load(Ordering::SeqCst), 3);

        // Verify results match expected pattern
        for result in results {
            let value = result.unwrap();
            assert!(value.starts_with("value-"));
        }
    }

    // Property-based testing helper
    #[tokio::test]
    async fn test_property_cache_consistency() {
        let cache = Arc::new(make_cache_table());

        // Randomly interleave operations
        let mut handles = vec![];

        for i in 0..50 {
            let cache_clone = cache.clone();
            let handle = tokio::task::spawn_local(async move {
                let key = (i % 10) as u64;

                match i % 4 {
                    0 => {
                        // Fetch
                        let _ = cache_clone
                            .get_or_fetch(&key, || {
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                tokio::task::spawn_local(async move {
                                    sleep(Duration::from_millis(10)).await;
                                    tx.send(Ok(format!("fetched-{}", key))).unwrap();
                                });
                                rx
                            })
                            .await;
                    }
                    1 => {
                        // Insert
                        cache_clone.insert(key, format!("inserted-{}", key));
                    }
                    2 => {
                        // Purge
                        cache_clone.purge(&key);
                    }
                    3 => {
                        // Purge_if
                        let _ = cache_clone.purge_if(|k| *k == key);
                    }
                    _ => unreachable!(),
                }
            });
            handles.push(handle);
        }

        // Wait for all operations
        for handle in handles {
            handle.await.unwrap();
        }

        // Cache should be in a consistent state (no panics = success)
        let _len = cache.get_len();
    }
}
