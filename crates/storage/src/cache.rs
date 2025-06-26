use std::{collections::HashMap, hash::Hash, num::NonZeroUsize, sync::Arc};

use lru::LruCache;
use parking_lot::RwLock;
use strata_db::{DbError, DbResult};
use tokio::sync::Mutex;

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
        tokio::task::block_in_place(move || {
            tokio::runtime::Handle::current().block_on(async {
                self.get_or_fetch(key, || {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let result = fetch_fn();
                    let _ = tx.send(result);
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
