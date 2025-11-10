//! Service implementation (placeholder for AsyncService pattern)

use crate::Prover;

/// Prover service (will implement AsyncService)
pub struct ProverService<P: Prover> {
    _phantom: std::marker::PhantomData<P>,
}

// TODO: Implement AsyncService trait from strata-service
// This will handle:
// - on_launch: Spawn worker pools
// - process_input: Handle commands (submit_task, get_status, etc.)
// - before_shutdown: Gracefully stop workers
