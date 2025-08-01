use std::{fmt::Debug, sync::Arc};

use tokio::sync::{mpsc, Mutex};

use super::*;

/// Adapter for using an [``Iterator``] as a [`SyncServiceInput``].
pub struct IterInput<I> {
    iter: I,
    closed: bool,
}

impl<I> IterInput<I> {
    pub fn new(iter: I) -> Self {
        Self {
            iter,
            closed: false,
        }
    }
}

impl<I: Iterator> ServiceInput for IterInput<I>
where
    I::Item: Debug,
{
    type Msg = I::Item;
}

impl<I: Iterator + Sync + Send + 'static> SyncServiceInput for IterInput<I>
where
    I::Item: Debug,
{
    fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        // We fuse it off ourselves just in case, it'd be weird not to.
        if self.closed {
            return Ok(None);
        }

        let item = self.iter.next();
        self.closed |= item.is_none();
        Ok(item)
    }
}

/// Adapter for using an async service input as a sync one.
pub struct SyncAsyncInput<I> {
    inner: I,
    handle: tokio::runtime::Handle,
}

impl<I> SyncAsyncInput<I> {
    pub fn new(inner: I, handle: tokio::runtime::Handle) -> Self {
        Self { inner, handle }
    }
}

impl<I: ServiceInput> ServiceInput for SyncAsyncInput<I>
where
    I::Msg: Debug,
{
    type Msg = I::Msg;
}

impl<I: AsyncServiceInput> SyncServiceInput for SyncAsyncInput<I> {
    fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        self.handle.block_on(self.inner.recv_next())
    }
}

/// Adapter for using a sync service input as an async one.
pub struct AsyncSyncInput<I> {
    // This is really annoying that it has to work this way.  Hopefully we won't
    // ever have to use this impl.
    inner: Arc<Mutex<I>>,
}

impl<I> AsyncSyncInput<I> {
    pub fn new(inner: I) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<I: ServiceInput> ServiceInput for AsyncSyncInput<I>
where
    I::Msg: Debug,
{
    type Msg = I::Msg;
}

impl<I: SyncServiceInput> AsyncServiceInput for AsyncSyncInput<I>
where
    I::Msg: Debug + Send,
{
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        let inner = self.inner.clone();
        let res = tokio::task::spawn_blocking(move || {
            let mut inner_lock = inner.blocking_lock();
            inner_lock.recv_next()
        })
        .await;

        match res {
            Ok(res) => res,
            // TODO don't use anyhow::bail! here
            Err(je) => {
                if je.is_cancelled() {
                    // How could this ever happen?
                    anyhow::bail!("wait for input cancelled")
                } else if je.is_panic() {
                    let _panic = je.into_panic(); // TODO do something with this
                    anyhow::bail!("input sleep paniced")
                } else {
                    anyhow::bail!("failed for unknown reason");
                }
            }
        }
    }
}

/// Adapter for using a mpsc receiver as a input.
// TODO convert to supporting any `Stream`?
pub struct TokioMpscInput<T> {
    rx: mpsc::Receiver<T>,
    closed: bool,
}

impl<T> TokioMpscInput<T> {
    pub fn new(rx: mpsc::Receiver<T>) -> Self {
        Self { rx, closed: false }
    }
}

impl<T: Debug> ServiceInput for TokioMpscInput<T> {
    type Msg = T;
}

impl<T: Debug + Send + 'static> AsyncServiceInput for TokioMpscInput<T> {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        // We fuse it off ourselves just in case, it'd be weird not to.
        if self.closed {
            return Ok(None);
        }

        let item = self.rx.recv().await;
        self.closed |= item.is_none();
        Ok(item)
    }
}
