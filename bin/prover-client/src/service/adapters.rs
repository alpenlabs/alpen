//! Adapters for integrating prover-client operators with paas traits
//!
//! This module provides adapter types that bridge between prover-client's
//! existing operator interfaces and the generic paas trait requirements:
//!
//! - `OperatorInputFetcher`: Adapts ProofInputFetcher to InputFetcher
//! - `ProofDbStorer`: Adapts ProofDBSled to ProofStorer
//!
//! Host resolution is now handled by `CentralizedHostResolver` in the `host_resolver` module.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_paas::{InputFetcher, ProofStorer};
use zkaleido::ProofReceiptWithMetadata;

use super::{proof_key_for, task::ProofTask};
use crate::{errors::ProvingTaskError, operators::ProofInputFetcher};

/// Error wrapper for proof storage operations
#[derive(Debug)]
pub(crate) struct ProofStorageError(anyhow::Error);

impl fmt::Display for ProofStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ProofStorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<anyhow::Error> for ProofStorageError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

/// Adapter that wraps an operator implementing ProofInputFetcher
/// to provide the paas InputFetcher trait
#[derive(Clone)]
pub(crate) struct OperatorInputFetcher<O> {
    operator: O,
    db: Arc<ProofDBSled>,
}

impl<O> OperatorInputFetcher<O> {
    pub(crate) fn new(operator: O, db: Arc<ProofDBSled>) -> Self {
        Self { operator, db }
    }
}

#[async_trait]
impl<O> InputFetcher<ProofTask> for OperatorInputFetcher<O>
where
    O: ProofInputFetcher + Clone + Send + Sync + 'static,
{
    type Input = O::Input;
    type Error = ProvingTaskError;

    async fn fetch_input(&self, program: &ProofTask) -> Result<Self::Input, Self::Error> {
        let proof_key = proof_key_for(program.0);
        self.operator.fetch_input(&proof_key, &self.db).await
    }
}

/// Adapter that wraps ProofDBSled to provide the paas ProofStorer trait
#[derive(Clone)]
pub(crate) struct ProofDbStorer {
    db: Arc<ProofDBSled>,
}

impl ProofDbStorer {
    pub(crate) fn new(db: Arc<ProofDBSled>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ProofStorer<ProofTask> for ProofDbStorer {
    type Error = ProofStorageError;

    async fn store_proof(
        &self,
        program: &ProofTask,
        proof: ProofReceiptWithMetadata,
    ) -> Result<(), Self::Error> {
        let proof_key = proof_key_for(program.0);
        self.db
            .put_proof(proof_key, proof)
            .map_err(|e| ProofStorageError(e.into()))?;
        Ok(())
    }
}
