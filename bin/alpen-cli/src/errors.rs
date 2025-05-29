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

/// Errors displayed to the user when using the Alpen CLI
#[derive(Debug)]
pub enum DisplayedError {
    /// Errors the use can address by updating configuration or providing expected input
    UserError(String, Box<dyn std::fmt::Debug>),
    /// Internal errors encountered when servicing user's request.
    InternalError(String, Box<dyn std::fmt::Debug>),
}

#[inline]
pub(crate) fn user_error<E>(msg: impl Into<String>) -> impl FnOnce(E) -> DisplayedError
where
    E: std::fmt::Debug + 'static,
{
    move |e| DisplayedError::UserError(msg.into(), Box::new(e))
}

#[inline]
pub(crate) fn internal_error<E>(msg: impl Into<String>) -> impl FnOnce(E) -> DisplayedError
where
    E: std::fmt::Debug + 'static,
{
    move |e| DisplayedError::InternalError(msg.into(), Box::new(e))
}

pub(crate) trait DisplayableError {
    type Output;
    fn user_error(self, msg: impl Into<String>) -> Result<Self::Output, DisplayedError>;
    fn internal_error(self, msg: impl Into<String>) -> Result<Self::Output, DisplayedError>;
}

impl<T, E: std::fmt::Debug + 'static> DisplayableError for Result<T, E> {
    type Output = T;

    #[inline]
    fn user_error(self, msg: impl Into<String>) -> Result<Self::Output, DisplayedError> {
        self.map_err(user_error(msg))
    }

    #[inline]
    fn internal_error(self, msg: impl Into<String>) -> Result<Self::Output, DisplayedError> {
        self.map_err(internal_error(msg))
    }
}

impl std::fmt::Display for DisplayedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayedError::UserError(msg, e) => {
                f.write_fmt(format_args!("User error: {msg}: {e:?}"))
            }
            DisplayedError::InternalError(msg, e) => {
                f.write_fmt(format_args!("Internal error: {msg}: {e:?}"))
            }
        }
    }
}
