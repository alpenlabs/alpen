use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use futures_util::task::AtomicWaker;

#[derive(Debug)]
pub(crate) struct PendingTasks {
    counter: AtomicUsize,
    waker: AtomicWaker,
}

impl PendingTasks {
    #[expect(clippy::missing_const_for_fn)]
    pub(crate) fn new(initial_count: usize) -> Self {
        Self {
            counter: AtomicUsize::new(initial_count),
            waker: AtomicWaker::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn current(&self) -> usize {
        self.counter.load(Ordering::SeqCst)
    }

    pub(crate) fn increment(&self) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn decrement(&self) {
        let prev = self.counter.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // Counter has reached zero
            self.waker.wake();
        }
    }

    pub(crate) const fn wait_for_zero(self: Arc<Self>) -> WaitForZero {
        WaitForZero {
            pending_tasks: self,
        }
    }
}

#[derive(Debug)]
pub(crate) struct WaitForZero {
    pending_tasks: Arc<PendingTasks>,
}

impl Future for WaitForZero {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.pending_tasks.counter.load(Ordering::SeqCst) == 0 {
            Poll::Ready(())
        } else {
            self.pending_tasks.waker.register(cx.waker());
            // Double-check the counter after registering the waker
            if self.pending_tasks.counter.load(Ordering::SeqCst) == 0 {
                self.pending_tasks.waker.wake();
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
}
