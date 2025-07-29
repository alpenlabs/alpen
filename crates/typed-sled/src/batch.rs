use std::marker::PhantomData;

use sled::Batch;

use crate::{KeyCodec, Schema, ValueCodec, error::Result};

/// Type-safe wrapper around a sled batch for atomic operations.
pub struct SledBatch<S: Schema> {
    pub(crate) inner: Batch,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledBatch<S> {
    /// Creates a new empty batch.
    pub fn new() -> Self {
        Self {
            inner: Batch::default(),
            _phantom: PhantomData,
        }
    }

    /// Adds an insert operation to the batch.
    pub fn insert(&mut self, key: S::Key, value: S::Value) -> Result<()> {
        let key = key.encode_key()?;
        let value = value.encode_value()?;
        self.inner.insert(key, value);
        Ok(())
    }

    /// Adds a remove operation to the batch.
    pub fn remove(&mut self, key: S::Key) -> Result<()> {
        let key = key.encode_key()?;
        self.inner.remove(key);
        Ok(())
    }
}

impl<S: Schema> Default for SledBatch<S> {
    fn default() -> Self {
        Self::new()
    }
}
