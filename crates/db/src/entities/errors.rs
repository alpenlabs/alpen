use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum EntityError {
    // No variants currently
}

pub type EntityResult<T> = Result<T, EntityError>;
