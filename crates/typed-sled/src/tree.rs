use std::{marker::PhantomData, sync::Arc};

use sled::Tree;

use crate::{KeyCodec, Schema, ValueCodec, error::Result};

pub struct SledTree<S: Schema> {
    inner: Arc<Tree>,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledTree<S> {
    pub fn new(inner: Arc<Tree>) -> Self {
        Self {
            inner,
            _phantom: PhantomData,
        }
    }

    pub fn put(&self, key: &S::Key, value: &S::Value) -> Result<()> {
        let key = key.encode_key()?;
        let value = value.encode_value()?;
        self.inner.insert(key, value)?;

        self.inner.flush()?;
        Ok(())
    }

    pub fn get(&self, key: &S::Key) -> Result<Option<S::Value>> {
        let key = key.encode_key()?;
        let val = self.inner.get(key)?;
        let val = val.as_deref();
        Ok(val.map(|v| S::Value::decode_value(v)).transpose()?)
    }

    pub fn remove(&self, key: &S::Key) -> Result<()> {
        let key = key.encode_key()?;
        self.inner.remove(key)?;

        self.inner.flush()?;
        Ok(())
    }
}
