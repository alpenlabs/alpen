//! Builder for creating prover service instances

use std::sync::Arc;

use crate::config::PaaSConfig;
use crate::error::{PaaSError, PaaSResult};
use crate::handle::ProverHandle;
use crate::Prover;

/// Builder for ProverService
pub struct ProverServiceBuilder<P: Prover> {
    prover: Option<Arc<P>>,
    config: Option<PaaSConfig<P::Backend>>,
}

impl<P: Prover> ProverServiceBuilder<P> {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            prover: None,
            config: None,
        }
    }

    /// Set the prover implementation
    pub fn with_prover(mut self, prover: Arc<P>) -> Self {
        self.prover = Some(prover);
        self
    }

    /// Set the configuration
    pub fn with_config(mut self, config: PaaSConfig<P::Backend>) -> Self {
        self.config = Some(config);
        self
    }

    /// Launch the service (placeholder)
    pub fn launch(self, _executor: &strata_tasks::TaskExecutor) -> PaaSResult<ProverHandle<P::TaskId>> {
        let _prover = self
            .prover
            .ok_or_else(|| PaaSError::Config("Prover not set".into()))?;
        let _config = self
            .config
            .ok_or_else(|| PaaSError::Config("Config not set".into()))?;

        // TODO: Create service state, launch service via ServiceBuilder, return handle
        Ok(ProverHandle::new())
    }
}

impl<P: Prover> Default for ProverServiceBuilder<P> {
    fn default() -> Self {
        Self::new()
    }
}
