//! Mock data provider for testing
//!
//! This module provides a mock implementation of `EthEeAcctDataProvider` that returns
//! default/minimal values. It's useful for:
//! - Unit tests
//! - Integration tests without a real database
//! - Development and prototyping
//! - Example code and documentation

use strata_acct_types::AccountId;
use strata_ee_acct_types::{CommitChainSegment, EeAccountState};
use strata_proofimpl_alpen_ee_acct::{DataProviderError, EthEeAcctDataProvider, Result, UpdateId};
use strata_snark_acct_types::UpdateOperationData;

/// Mock data provider that returns default values
///
/// This implementation is useful for testing and development when you don't have
/// a real database or when SSZ encoding/decoding is not yet implemented.
///
/// All methods return default or minimal values that satisfy the type requirements
/// but may not represent valid proof inputs.
#[derive(Clone, Copy, Debug, Default)]
pub struct MockDataProvider;

impl MockDataProvider {
    /// Create a new mock data provider
    pub fn new() -> Self {
        Self
    }
}

impl EthEeAcctDataProvider for MockDataProvider {
    fn fetch_ee_account_state(&self, _account_id: AccountId) -> Result<EeAccountState> {
        // Return a default EeAccountState
        // This will fail if used for actual proof generation until SSZ is implemented
        Ok(EeAccountState::default())
    }

    fn fetch_update_operation(&self, _update_id: UpdateId) -> Result<UpdateOperationData> {
        // Return a default UpdateOperationData
        Ok(UpdateOperationData::default())
    }

    fn fetch_chain_segments(&self, _commit_blkids: &[[u8; 32]]) -> Result<Vec<CommitChainSegment>> {
        // Return an empty vector
        // Actual proofs will need real chain segments
        Ok(Vec::new())
    }

    fn fetch_previous_header(&self, _exec_blkid: [u8; 32]) -> Result<Vec<u8>> {
        // Return empty bytes
        // Actual proofs will need the real previous header
        Ok(Vec::new())
    }

    fn fetch_partial_state(&self, _exec_blkid: [u8; 32]) -> Result<Vec<u8>> {
        // Return empty bytes
        // Actual proofs will need the real partial state
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_provider_creation() {
        let _provider = MockDataProvider::new();
        let _provider2 = MockDataProvider::default();
    }

    #[test]
    fn test_mock_provider_methods() {
        let provider = MockDataProvider::new();

        // Test that all methods return Ok (even if with default values)
        assert!(provider.fetch_ee_account_state(AccountId::default()).is_ok());
        assert!(provider.fetch_update_operation(0).is_ok());
        assert!(provider.fetch_chain_segments(&[]).is_ok());
        assert!(provider.fetch_previous_header([0u8; 32]).is_ok());
        assert!(provider.fetch_partial_state([0u8; 32]).is_ok());
    }
}
