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
    use std::sync::atomic::{AtomicU32, Ordering};

    use strata_db::DbError;
    use tokio::sync::oneshot;

    use super::*;

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

    /// This test validates the [`Arc::ptr_eq`] fix for the race condition where:
    /// 1. A cache slot is created and fails
    /// 2. Error handling tries to remove the slot
    /// 3. The removal should only happen if it's the same slot ([`Arc::ptr_eq`] check)
    #[tokio::test]
    async fn test_concurrent_race_condition() {
        let cache = CacheTable::<u64, u64>::new(10.try_into().unwrap());
        let fetch_count = Arc::new(AtomicU32::new(0));

        let key = 42u64;
        let success_value = 100u64;

        // First, populate the cache with a successful value
        let count1 = Arc::clone(&fetch_count);
        let result1 = cache
            .get_or_fetch(&key, || {
                count1.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = oneshot::channel();
                tx.send(Ok(success_value)).expect("send success");
                rx
            })
            .await;

        assert!(result1.is_ok());
        assert_eq!(result1.unwrap(), success_value);
        assert_eq!(cache.get_len(), 1);

        // Now try to fetch the same key again - this should hit the cache
        let count2 = Arc::clone(&fetch_count);
        let result2 = cache
            .get_or_fetch(&key, || {
                count2.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = oneshot::channel();
                // This should not be called since we have a cache hit
                tx.send(Err(DbError::Busy)).expect("should not be called");
                rx
            })
            .await;

        assert!(result2.is_ok());
        assert_eq!(result2.unwrap(), success_value);

        // Critical test: Verify the successful value is still cached and not removed
        // This validates that the `Arc::ptr_eq` check prevents wrong slot removal
        let final_count = fetch_count.load(Ordering::SeqCst);
        assert_eq!(
            final_count, 1,
            "Only the first fetch should have been executed"
        );
        assert_eq!(cache.get_len(), 1, "Cache should still contain the entry");
    }

    /// This test validates the Arc::ptr_eq fix for the race condition where:
    /// 1. A successful value is cached
    /// 2. A subsequent failed fetch should NOT remove the successful cached value
    #[tokio::test]
    async fn test_error_slot_removal() {
        let cache = CacheTable::<u64, u64>::new(10.try_into().unwrap());

        // First, populate the cache with a successful value
        let success_result = cache
            .get_or_fetch(&42, || {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tx.send(Ok(100u64)).expect("send success");
                rx
            })
            .await;

        assert!(success_result.is_ok());
        assert_eq!(success_result.unwrap(), 100u64);
        assert_eq!(cache.get_len(), 1, "Successful entry should be cached");

        // Now try to fetch the same key again, but have it fail
        // This simulates a race condition scenario where the second fetch fails
        // but should NOT remove the successful slot due to Arc::ptr_eq check
        let failed_result = cache
            .get_or_fetch(&42, || {
                // This should not be called since we have a cache hit
                panic!("Should not fetch again - value should be cached!");
            })
            .await;

        // The second fetch should return the cached successful value, not fail
        assert!(failed_result.is_ok());
        assert_eq!(failed_result.unwrap(), 100u64);

        // Critical test: The successful value should still be cached
        // This validates that the Arc::ptr_eq check prevents wrong slot removal
        assert_eq!(cache.get_len(), 1, "Cache should still contain the entry");
    }

    /// This test validates that the blocking cache works correctly
    /// and that failed fetches don't interfere with successful ones
    #[test]
    fn test_race_condition_blocking() {
        use std::sync::{
            atomic::{AtomicU32, Ordering},
            Arc,
        };

        let cache = CacheTable::<u64, u64>::new(3.try_into().unwrap());
        let fetch_count = Arc::new(AtomicU32::new(0));

        // First, try a fetch that will fail
        let count1 = Arc::clone(&fetch_count);
        let result1 = cache.get_or_fetch_blocking(&42, || {
            count1.fetch_add(1, Ordering::SeqCst);
            Err(DbError::Busy)
        });

        assert!(result1.is_err());
        assert_eq!(cache.get_len(), 0, "Failed fetch should not cache anything");

        // Now try a fetch that will succeed with the same key
        let count2 = Arc::clone(&fetch_count);
        let result2 = cache.get_or_fetch_blocking(&42, || {
            count2.fetch_add(1, Ordering::SeqCst);
            Ok(200u64)
        });

        assert!(result2.is_ok());
        assert_eq!(result2.unwrap(), 200u64);
        assert_eq!(cache.get_len(), 1, "Successful fetch should be cached");

        // Verify subsequent fetch hits the cache
        let cached_result = cache
            .get_or_fetch_blocking(&42, || panic!("Should not fetch - value should be cached!"))
            .expect("cached value should be available");

        assert_eq!(cached_result, 200u64);

        // Both fetch functions should have been called
        let final_count = fetch_count.load(Ordering::SeqCst);
        assert_eq!(
            final_count, 2,
            "Both fetch attempts should have been called"
        );
    }
}
