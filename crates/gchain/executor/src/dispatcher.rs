/// Abstraction for how we execute low-level work.
///
/// Will be made more general in the future to support sending off to a threadpool.
pub trait Dispatcher {
    /// Dispatches a work operation to execute.
    fn dispatch(&self, work: impl FnOnce() + Send + 'static);
}

/// Dispatches work onto the current thread, blocking until complete.
pub struct InlineDispatcher;

impl Dispatcher for InlineDispatcher {
    fn dispatch(&self, work: impl FnOnce() + Send + 'static) {
        work()
    }
}
