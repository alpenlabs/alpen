//! Sync wrapper around [`tokio::sync::watch::Receiver`].
//!
//! All methods are direct wrappers around their counterparts.

use tokio::{runtime::Handle, sync::watch};

#[derive(Debug)]
pub struct SyncReceiver<T> {
    rx: watch::Receiver<T>,
    rt: Handle,
}

impl<T> SyncReceiver<T> {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(rx: watch::Receiver<T>, rt: Handle) -> Self {
        Self { rx, rt }
    }

    pub fn borrow(&mut self) -> watch::Ref<'_, T> {
        self.rx.borrow()
    }

    pub fn borrow_and_update(&mut self) -> watch::Ref<'_, T> {
        self.rx.borrow_and_update()
    }

    pub fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.rt.block_on(self.rx.changed())
    }

    pub fn wait_for(
        &mut self,
        f: impl Fn(&'_ T) -> bool,
    ) -> Result<watch::Ref<'_, T>, watch::error::RecvError> {
        self.rt.block_on(self.rx.wait_for(f))
    }
}
