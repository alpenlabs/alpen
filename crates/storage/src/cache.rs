//! Generic cache utility for what we're inserting into the database.

use std::{collections::HashMap, hash::Hash, num::NonZeroUsize, sync::Arc};

use lru::LruCache;
use parking_lot::RwLock;
use strata_db::{DbError, DbResult};
use tokio::{
    runtime,
    sync::{oneshot, Mutex},
    task::block_in_place,
};

use crate::exec::DbRecv;

pub(crate) struct CacheTable<K, V> {
    /// The actual cache.
    cache: RwLock<LruCache<K, V>>,

    /// One mutex per key to prevent duplicate fetches.
    /// Created when inserting, cleaned up when the entry is purged.
    /// Not sure if trying to clean this up after fetch complete is worthwhile. Seems to add a
    /// bunch of complex logic whereas keeping it won't have much memory footprint.
    fetch_mutexes: RwLock<HashMap<K, Arc<Mutex<()>>>>,
}

impl<K, V> CacheTable<K, V>
where
    K: Clone + Eq + Hash + Send + Sync,
    V: Clone + Send + Sync,
{
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            cache: RwLock::new(LruCache::new(capacity)),
            fetch_mutexes: RwLock::new(HashMap::new()),
        }
    }

    fn get(&self, key: &K) -> Option<V> {
        let mut cache_guard = self.cache.write();
        cache_guard.get(key).cloned()
    }

    pub(crate) fn insert(&self, key: K, value: V) {
        let mut cache_guard = self.cache.write();
        cache_guard.put(key, value);
    }

    pub(crate) fn purge(&self, key: &K) {
        let mut cache_guard = self.cache.write();
        cache_guard.pop(key);
        // Remoe fetch mutex if any
        let mut fetch_guard = self.fetch_mutexes.write();
        fetch_guard.remove(key);
    }

    pub(crate) fn purge_if(&self, mut pred: impl FnMut(&K) -> bool) -> usize {
        let mut cache_guard = self.cache.write();
        let keys_to_remove = cache_guard
            .iter()
            .map(|(k, _v)| k)
            .filter(|k| pred(k))
            .cloned()
            .collect::<Vec<_>>();

        let mut fetch_guard = self.fetch_mutexes.write();
        keys_to_remove.iter().for_each(|k| {
            cache_guard.pop(k);
            fetch_guard.remove(k);
        });
        keys_to_remove.len()
    }

    pub(crate) fn clear(&self) -> usize {
        let len = self.get_len();
        let mut cache_guard = self.cache.write();
        cache_guard.clear();

        // Remove mutexes
        let mut fetch_guard = self.fetch_mutexes.write();
        fetch_guard.clear();

        len
    }

    pub(crate) fn get_len(&self) -> usize {
        self.cache.read().len()
    }

    pub(crate) async fn get_or_fetch(
        &self,
        key: &K,
        fetch_fn: impl Fn() -> DbRecv<V>,
    ) -> DbResult<V> {
        if let Some(v) = self.get(key) {
            return Ok(v);
        }

        // Get the mutex
        let mutex = {
            let mut fetch_guard = self.fetch_mutexes.write();
            fetch_guard
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        // If not, need to fetch
        let _guard = mutex.lock().await;

        // After acquiring the lock, check for the value
        if let Some(value) = self.get(key) {
            // Value present, no need to call fetch_fn
            return Ok(value);
        }

        // The cache value is not set, do fetch_fn
        match fetch_fn().await {
            Ok(Ok(value)) => {
                self.insert(key.clone(), value.clone());
                Ok(value)
            }
            Ok(Err(e)) => {
                // Just cleanup mutex and return the error. Don't do anything to the cache
                Err(e)
            }
            Err(e) => Err(DbError::Other(e.to_string())),
        }
    }

    pub(crate) fn get_or_fetch_blocking(
        &self,
        key: &K,
        fetch_fn: impl Fn() -> DbResult<V>,
    ) -> DbResult<V> {
        if let Some(v) = self.get(key) {
            return Ok(v);
        }
        block_in_place(move || {
            runtime::Handle::current().block_on(async {
                self.get_or_fetch(key, || {
                    let (tx, rx) = oneshot::channel();
                    let _ = tx.send(fetch_fn());
                    rx
                })
                .await
            })
        })
    }
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_basic_blocking() {
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
mod concurrent_tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use tokio::time::sleep;

    use super::*;

    fn helper_fetch_fn(
        count: Arc<AtomicUsize>,
        return_value: DbResult<u64>,
    ) -> impl Fn() -> oneshot::Receiver<DbResult<u64>> {
        move || {
            let (tx, rx) = tokio::sync::oneshot::channel();
            count.fetch_add(1, Ordering::SeqCst);
            tx.send(return_value.clone()).expect("send value");
            rx
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_fetch_prevention() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(10.try_into().unwrap()));
        let fetch_count = Arc::new(AtomicUsize::new(0));

        let tasks: Vec<_> = (0..10)
            .map(|_| {
                let cache = cache.clone();
                let fetch_count = fetch_count.clone();
                tokio::spawn(async move {
                    cache
                        .get_or_fetch(&42, helper_fetch_fn(fetch_count, Ok(100)))
                        .await
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(tasks).await;

        // All tasks should succeed with same value
        for result in results {
            assert_eq!(result.unwrap().unwrap(), 100);
        }

        // Only one fetch should have occurred
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_different_keys() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(10.try_into().unwrap()));
        let fetch_count = Arc::new(AtomicUsize::new(0));

        let tasks: Vec<_> = (0..5)
            .map(|i| {
                let cache = cache.clone();
                let fetch_count = fetch_count.clone();
                tokio::spawn(async move {
                    cache
                        .get_or_fetch(&i, helper_fetch_fn(fetch_count, Ok(i * 10)))
                        .await
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(tasks).await;

        // All tasks should succeed with correct values
        for (i, result) in results.into_iter().enumerate() {
            assert_eq!(result.unwrap().unwrap(), (i as u64) * 10);
        }

        // One fetch per key
        assert_eq!(fetch_count.load(Ordering::SeqCst), 5);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_fetch_error_handling() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(10.try_into().unwrap()));
        let fetch_count = Arc::new(AtomicUsize::new(0));

        let tasks: Vec<_> = (0..5)
            .map(|_| {
                let cache = cache.clone();
                let fetch_count = fetch_count.clone();
                tokio::spawn(async move {
                    cache
                        .get_or_fetch(&42, helper_fetch_fn(fetch_count, Err(DbError::Busy)))
                        .await
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(tasks).await;

        // All tasks should fail with same error
        for result in results {
            assert!(matches!(result.unwrap(), Err(DbError::Busy)));
        }

        // Only one fetch should occur even on error
        assert_eq!(fetch_count.load(Ordering::SeqCst), 5);

        // Cache should remain empty
        assert_eq!(cache.get_len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_purge_during_fetch() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(10.try_into().unwrap()));

        // Start a slow fetch
        let cache_clone = cache.clone();
        let fetch_task = tokio::spawn(async move {
            cache_clone
                .get_or_fetch(&42, || {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    tokio::spawn(async move {
                        sleep(Duration::from_millis(50)).await;
                        tx.send(Ok(100)).expect("send value");
                    });
                    rx
                })
                .await
        });

        // Purge while fetch is ongoing
        sleep(Duration::from_millis(10)).await;
        cache.purge(&42);

        // Fetch should still complete successfully
        let result = fetch_task.await.unwrap();
        assert_eq!(result.unwrap(), 100);

        // Value should be in cache after fetch completes
        assert_eq!(cache.get(&42), Some(100));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_lru_eviction_under_concurrency() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(3.try_into().unwrap()));

        // Fill cache concurrently
        let tasks: Vec<_> = (0..10)
            .map(|i| {
                let cache = cache.clone();
                tokio::spawn(async move {
                    cache.insert(i, i * 10);
                })
            })
            .collect();

        futures::future::join_all(tasks).await;

        // Cache should only have 3 items
        assert_eq!(cache.get_len(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_clear_operations() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(10.try_into().unwrap()));

        // Insert some values
        for i in 0..5 {
            cache.insert(i, i);
        }

        // Concurrent clear and access
        let cache_clone = cache.clone();
        let clear_task = tokio::spawn(async move { cache_clone.clear() });
        let access_tasks: Vec<_> = (0..5)
            .map(|i| {
                let cache = cache.clone();
                tokio::spawn(async move { cache.get(&i) })
            })
            .collect();

        let clear_count = clear_task.await.unwrap();
        let _access_results: Vec<_> = futures::future::join_all(access_tasks).await;

        // Clear should have removed some items
        assert!(clear_count > 0);

        // Final state should be consistent
        assert_eq!(cache.get_len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_mixed_async_blocking_access() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(10.try_into().unwrap()));
        let fetch_count = Arc::new(AtomicUsize::new(0));

        // Mix of async and blocking operations
        let async_task = {
            let cache = cache.clone();
            let fetch_count = fetch_count.clone();
            tokio::spawn(async move {
                cache
                    .get_or_fetch(&42, helper_fetch_fn(fetch_count, Ok(100)))
                    .await
            })
        };

        let blocking_task = {
            let cache = cache.clone();
            let fetch_count = fetch_count.clone();
            tokio::spawn(async move {
                tokio::task::spawn_blocking(move || {
                    cache.get_or_fetch_blocking(&42, || {
                        fetch_count.fetch_add(1, Ordering::SeqCst);
                        Ok(200)
                    })
                })
                .await
                .unwrap()
            })
        };

        let (async_result, blocking_result) = tokio::join!(async_task, blocking_task);

        // Both should succeed
        let async_val = async_result.unwrap().unwrap();
        let blocking_val = blocking_result.unwrap().unwrap();

        // One should get the cached value from the other
        assert!(async_val == 100 || async_val == 200);
        assert!(blocking_val == 100 || blocking_val == 200);
        // Both values should be the same
        assert_eq!(async_val, blocking_val);

        // Only one fetch should occur
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn test_high_concurrency_stress() {
        let cache = Arc::new(CacheTable::<u64, u64>::new(50.try_into().unwrap()));
        let fetch_count = Arc::new(AtomicUsize::new(0));

        // 100 concurrent operations on 10 different keys
        let tasks: Vec<_> = (0..100)
            .map(|i| {
                let cache = cache.clone();
                let fetch_count = fetch_count.clone();
                let key = (i % 10) as u64;

                tokio::spawn(async move {
                    cache
                        .get_or_fetch(&key, helper_fetch_fn(fetch_count, Ok(key * 100)))
                        .await
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(tasks).await;

        // All operations should succeed
        for result in results {
            assert!(result.unwrap().is_ok());
        }

        // Should have exactly 10 fetches (one per unique key)
        assert_eq!(fetch_count.load(Ordering::SeqCst), 10);

        // Cache should contain all 10 keys
        assert_eq!(cache.get_len(), 10);
    }
}
