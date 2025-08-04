use std::{fmt::Debug, sync::Arc};

use futures::{
    pin_mut,
    stream::{Stream, StreamExt},
};
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
///
/// This is needed because [``mpsc::Receiver``] does not natively implement
/// [``Stream``] and it avoids having to use the Tokio wrapper.
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

/// This impl is technically redundant since we can use the type as an
/// [``Iterator``], but someone might find it useful and it's easy enough to
/// implement.
impl<T: Debug + Send + 'static> SyncServiceInput for TokioMpscInput<T> {
    fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        // We fuse it off ourselves just in case, it'd be weird not to.
        if self.closed {
            return Ok(None);
        }

        let item = self.rx.blocking_recv();
        self.closed |= item.is_none();
        Ok(item)
    }
}

/// Adapter for using an arbitrary [``Stream``] impl as an input.
pub struct StreamInput<S> {
    stream: S,
    closed: bool,
}

impl<S> StreamInput<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            closed: false,
        }
    }
}

impl<S: Stream> ServiceInput for StreamInput<S>
where
    S::Item: Debug,
{
    type Msg = S::Item;
}

impl<S: Stream + Unpin + Sync + Send + 'static> AsyncServiceInput for StreamInput<S>
where
    S::Item: Debug,
{
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        // We fuse it off ourselves just in case, it'd be weird not to.
        if self.closed {
            return Ok(None);
        }

        let item = self.stream.next().await;
        self.closed |= item.is_none();
        Ok(item)
    }
}

#[cfg(test)]
mod tests {
    use futures::stream;
    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn test_stream_input() {
        let v = 3;
        let stream = stream::repeat(v);
        let mut inp = StreamInput::new(stream);

        let rv = inp
            .recv_next()
            .await
            .expect("test: recv input")
            .expect("test: have input");

        assert_eq!(rv, v, "test: input match");
    }

    #[tokio::test]
    async fn test_mpsc_input_async() {
        let v = 3;

        let (tx, rx) = mpsc::channel(10);
        let mut inp = TokioMpscInput::new(rx);

        tx.send(v).await.expect("test: send input");

        let rv = AsyncServiceInput::recv_next(&mut inp)
            .await
            .expect("test: recv input")
            .expect("test: have input");

        assert_eq!(rv, v, "test: input match");
    }

    #[test]
    fn test_mpsc_input_blocking() {
        let v = 3;

        let (tx, rx) = mpsc::channel(10);
        let mut inp = TokioMpscInput::new(rx);

        tx.blocking_send(v).expect("test: send input");

        let rv = SyncServiceInput::recv_next(&mut inp)
            .expect("test: recv input")
            .expect("test: have input");

        assert_eq!(rv, v, "test: input match");
    }
}
