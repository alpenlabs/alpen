//! Bridge transaction data structures
//!
//! This module contains the core data structures for bridge operations.

/// Bitcoind configuration
#[derive(Debug, Clone)]
pub(crate) struct BitcoinDConfig {
    pub bitcoind_url: String,
    pub bitcoind_user: String,
    pub bitcoind_password: String,
}
