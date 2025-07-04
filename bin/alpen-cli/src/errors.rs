pub(crate) type BoxedInner = dyn std::error::Error + Send + Sync;
pub(crate) type BoxedErr = Box<BoxedInner>;

/// This indicates runtime failure in the underlying platform storage system. The details of the
/// failure can be retrieved from the attached platform error.
#[derive(Debug)]
#[allow(unused)]
pub struct PlatformFailure(BoxedErr);

impl PlatformFailure {
    pub fn new<E>(e: E) -> Self
    where
        E: Into<BoxedErr>,
    {
        Self(e.into())
    }
}

/// This indicates that the underlying secure storage holding saved items could not be accessed.
/// Typically this is because of access rules in the platform; for example, it might be that the
/// credential store is locked. The underlying platform error will typically give the reason.
#[derive(Debug)]
#[allow(unused)]
pub struct NoStorageAccess(BoxedErr);

impl NoStorageAccess {
    pub fn new<E>(e: E) -> Self
    where
        E: Into<BoxedErr>,
    {
        Self(e.into())
    }
}
