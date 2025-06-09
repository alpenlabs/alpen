use strata_db::DbError;

/// Unified error type wrapping library errors and CLI‑specific issues.
#[derive(Debug)]
pub enum DbtoolError {
    Io(std::io::Error),
    Db(String),
}

impl From<std::io::Error> for DbtoolError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for DbtoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // <- we *use* the inner error here, so the field is no longer “dead”
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Db(s) => write!(f, "DB error: {s}"),
        }
    }
}

impl std::error::Error for DbtoolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Db(_) => None,
        }
    }
}

impl From<DbError> for DbtoolError {
    fn from(e: DbError) -> Self {
        DbtoolError::Db(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DbtoolError>;
