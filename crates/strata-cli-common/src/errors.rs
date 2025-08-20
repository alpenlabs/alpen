/// Errors displayed to the user when using the CLI
#[derive(Debug)]
pub enum DisplayedError {
    /// Errors the use can address by updating configuration or providing expected input
    UserError(String, Box<dyn std::fmt::Debug>),
    /// Internal errors encountered when servicing user's request.
    InternalError(String, Box<dyn std::fmt::Debug>),
}

#[inline]
pub fn user_error<E>(msg: impl Into<String>) -> impl FnOnce(E) -> DisplayedError
where
    E: std::fmt::Debug + 'static,
{
    move |e| DisplayedError::UserError(msg.into(), Box::new(e))
}

#[inline]
pub fn internal_error<E>(msg: impl Into<String>) -> impl FnOnce(E) -> DisplayedError
where
    E: std::fmt::Debug + 'static,
{
    move |e| DisplayedError::InternalError(msg.into(), Box::new(e))
}

pub trait DisplayableError {
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
