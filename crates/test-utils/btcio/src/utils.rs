use tokio::task::block_in_place;

/// If we're already in a tokio runtime, we'll block in place. Otherwise, we'll create a new
/// runtime.
pub(crate) fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    // Handle case if we're already in an tokio runtime.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        block_in_place(|| handle.block_on(fut))
    } else {
        // Otherwise create a new runtime.
        let rt = tokio::runtime::Runtime::new().expect("Failed to create a new runtime");
        rt.block_on(fut)
    }
}
