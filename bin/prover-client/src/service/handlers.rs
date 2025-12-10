//! ProofHandler implementations for all proof types
//!
//! This module defines type aliases for the three proof handlers (Checkpoint, OLStf, EvmEe)
//! using the generic RemoteProofHandler from paas with prover-client-specific adapters.

use std::sync::Arc;

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::RemoteProofHandler;
use strata_tasks::TaskExecutor;

use super::{
    adapters::{OperatorInputFetcher, ProofDbStorer},
    host_resolver::CentralizedHostResolver,
};
use crate::operators::{CheckpointOperator, EvmEeOperator, OLStfOperator};

/// Type alias for Checkpoint proof handler
///
/// Uses the generic RemoteProofHandler from paas with:
/// - CheckpointOperator wrapped in OperatorInputFetcher
/// - ProofDbStorer for persistence
/// - CentralizedHostResolver for host resolution (single source of truth)
pub(crate) type CheckpointHandler = RemoteProofHandler<
    super::task::ProofTask,
    OperatorInputFetcher<CheckpointOperator>,
    ProofDbStorer,
    CentralizedHostResolver,
    strata_proofimpl_checkpoint::program::CheckpointProgram,
>;

/// Type alias for OL STF proof handler
pub(crate) type OLStfHandler = RemoteProofHandler<
    super::task::ProofTask,
    OperatorInputFetcher<OLStfOperator>,
    ProofDbStorer,
    CentralizedHostResolver,
    strata_proofimpl_ol_stf::program::OLStfProgram,
>;

/// Type alias for EVM EE STF proof handler
pub(crate) type EvmEeStfHandler = RemoteProofHandler<
    super::task::ProofTask,
    OperatorInputFetcher<EvmEeOperator>,
    ProofDbStorer,
    CentralizedHostResolver,
    strata_proofimpl_evm_ee_stf::program::EvmEeProgram,
>;

/// Create a new CheckpointHandler
pub(crate) fn new_checkpoint_handler(
    operator: CheckpointOperator,
    db: Arc<ProofDBSled>,
    executor: TaskExecutor,
) -> CheckpointHandler {
    let fetcher = OperatorInputFetcher::new(operator, db.clone());
    let storer = ProofDbStorer::new(db);
    let resolver = CentralizedHostResolver;
    RemoteProofHandler::new(fetcher, storer, resolver, executor)
}

/// Create a new OLStfHandler
pub(crate) fn new_ol_stf_handler(
    operator: OLStfOperator,
    db: Arc<ProofDBSled>,
    executor: TaskExecutor,
) -> OLStfHandler {
    let fetcher = OperatorInputFetcher::new(operator, db.clone());
    let storer = ProofDbStorer::new(db);
    let resolver = CentralizedHostResolver;
    RemoteProofHandler::new(fetcher, storer, resolver, executor)
}

/// Create a new EvmEeStfHandler
pub(crate) fn new_evm_ee_stf_handler(
    operator: EvmEeOperator,
    db: Arc<ProofDBSled>,
    executor: TaskExecutor,
) -> EvmEeStfHandler {
    let fetcher = OperatorInputFetcher::new(operator, db.clone());
    let storer = ProofDbStorer::new(db);
    let resolver = CentralizedHostResolver;
    RemoteProofHandler::new(fetcher, storer, resolver, executor)
}
