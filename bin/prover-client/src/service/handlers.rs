//! ProofHandler implementations for all proof types
//!
//! This module defines type aliases for the three proof handlers (Checkpoint, ClStf, EvmEe)
//! using the generic RemoteProofHandler from paas with prover-client-specific adapters.

use std::sync::Arc;

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::RemoteProofHandler;
use strata_proofimpl_checkpoint::program::CheckpointProgram;
use strata_proofimpl_cl_stf::program::ClStfProgram;
use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;
use strata_tasks::TaskExecutor;

use super::{
    adapters::{OperatorInputFetcher, ProofDbStorer},
    host_resolver::CentralizedHostResolver,
};
use crate::operators::{CheckpointOperator, ClStfOperator, EvmEeOperator};

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
    CheckpointProgram,
>;

/// Type alias for CL STF proof handler
pub(crate) type ClStfHandler = RemoteProofHandler<
    super::task::ProofTask,
    OperatorInputFetcher<ClStfOperator>,
    ProofDbStorer,
    CentralizedHostResolver,
    ClStfProgram,
>;

/// Type alias for EVM EE STF proof handler
pub(crate) type EvmEeStfHandler = RemoteProofHandler<
    super::task::ProofTask,
    OperatorInputFetcher<EvmEeOperator>,
    ProofDbStorer,
    CentralizedHostResolver,
    EvmEeProgram,
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

/// Create a new ClStfHandler
pub(crate) fn new_cl_stf_handler(
    operator: ClStfOperator,
    db: Arc<ProofDBSled>,
    executor: TaskExecutor,
) -> ClStfHandler {
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
