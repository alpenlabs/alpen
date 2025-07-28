use std::marker::PhantomData;

use sled::Batch;

use crate::{KeyCodec, Schema, ValueCodec, error::Result};

/// Typesafe wrapper to a sled [`Batch`].
pub struct SledBatch<S: Schema> {
    pub(crate) inner: Batch,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledBatch<S> {
    pub fn new() -> Self {
        Self {
            inner: Batch::default(),
            _phantom: PhantomData,
        }
    }

    pub fn insert(&mut self, key: S::Key, value: S::Value) -> Result<()> {
        let key = key.encode_key()?;
        let value = value.encode_value()?;
        self.inner.insert(key, value);
        Ok(())
    }

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
