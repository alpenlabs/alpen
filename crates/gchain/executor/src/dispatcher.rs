/// Abstraction for how we execute low-level work.
///
/// Will be made more general in the future to support sending off to a threadpool.
///
/// The work is boxed rather than taken as `impl FnOnce` so that the trait stays
/// dyn-compatible, which is what lets an executor hold whichever dispatcher it
/// was configured with behind a `dyn Dispatcher`.
pub trait Dispatcher {
    /// Dispatches a work operation to execute.
    fn dispatch(&self, work: Box<dyn FnOnce() + Send + 'static>);
}

/// Dispatches work onto the current thread, blocking until complete.
pub struct InlineDispatcher;

impl Dispatcher for InlineDispatcher {
    fn dispatch(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        work()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn test_inline_dispatcher_runs_work() {
        let calls = Arc::new(AtomicUsize::new(0));
        let dispatcher = InlineDispatcher;

        let counter = Arc::clone(&calls);
        dispatcher.dispatch(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// The point of boxing the work is that a dispatcher can be held as a trait
    /// object.
    #[test]
    fn test_dispatcher_is_dyn_compatible() {
        let calls = Arc::new(AtomicUsize::new(0));
        let dispatcher: Arc<dyn Dispatcher> = Arc::new(InlineDispatcher);

        let counter = Arc::clone(&calls);
        dispatcher.dispatch(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
