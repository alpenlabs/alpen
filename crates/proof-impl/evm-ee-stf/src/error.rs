//! Error types for EVM Execution Environment State Transition Function (EE-STF) proof
//! implementation.

use alloy_primitives::Address;
use revm_primitives::alloy_primitives::U256;
use thiserror::Error;

/// EVM EE-STF proof implementation operations errors.
#[derive(Error, Debug)]
pub enum EvmEeStfError {
    /// Database operations error.
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    /// Merkle Patricia Trie operations error.
    #[error("MPT error: {0}")]
    Mpt(#[from] strata_mpt::Error),

    /// Account operations error.
    #[error("Account error: {0}")]
    Account(#[from] AccountError),

    /// Validation error.
    #[error("Validation error: {message}")]
    Validation { message: String },

    /// Block processing error.
    #[error("Block processing error: {message}")]
    BlockProcessing { message: String },
}

/// Database operations errors.
#[derive(Error, Debug)]
pub enum DatabaseError {
    /// Account not found.
    #[error("Account not found: {address}")]
    AccountNotFound { address: Address },

    /// Storage slot not found for the specified account and index.
    #[error("Storage slot not found for account {address} at index {index}")]
    StorageSlotNotFound { address: Address, index: U256 },

    /// Database operation failed.
    #[error("Database operation failed: {message}")]
    OperationFailed { message: String },

    /// Error when committing changes to the database.
    #[error("Failed to commit changes to database: {message}")]
    CommitFailed { message: String },
}

/// Account operations errors.
#[derive(Error, Debug)]
pub enum AccountError {
    /// Failed to increase account balance.
    #[error("Failed to increase balance for account {address}: {message}")]
    BalanceIncreaseFailed { address: Address, message: String },

    /// Inconsistent account state.
    #[error("Inconsistent account state for {address}: {message}")]
    InconsistentState { address: Address, message: String },

    /// Invalid account data.
    #[error("Invalid account data for {address}: {message}")]
    InvalidData { address: Address, message: String },
}

/// Results for EVM EE-STF proof implementation operations.
pub type EvmEeStfResult<T> = Result<T, EvmEeStfError>;

/// Results for Database operations.
pub type DatabaseResult<T> = Result<T, DatabaseError>;

/// Results for Account operations.
pub type AccountResult<T> = Result<T, AccountError>;
