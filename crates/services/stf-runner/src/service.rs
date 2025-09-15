use serde::Serialize;
use strata_service::{AsyncService, Response, Service, ServiceState};

/// Service implementation for the State Transition Function (STF) runner
#[derive(Debug, Clone)]
pub struct StfRunnerService {}

/// Messages that can be sent to the STF runner service
#[derive(Debug, Clone)]
pub enum StfRunnerMessage {}

impl Service for StfRunnerService {
    type State = StfRunnerState;
    type Msg = StfRunnerMessage;

    type Status = StfRunnerStatus;

    fn get_status(_s: &Self::State) -> Self::Status {
        StfRunnerStatus {}
    }
}

impl AsyncService for StfRunnerService {
    async fn process_input(
        _state: &mut Self::State,
        _input: &Self::Msg,
    ) -> anyhow::Result<Response> {
        Ok(Response::Continue)
    }

    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn before_shutdown(
        _state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Internal state for the STF runner service
#[derive(Debug, Clone)]
pub struct StfRunnerState {}

impl ServiceState for StfRunnerState {
    fn name(&self) -> &str {
        "stf-runner"
    }
}

/// Status information for the STF runner service
#[derive(Debug, Clone, Serialize)]
pub struct StfRunnerStatus {}

impl StfRunnerService {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for StfRunnerService {
    fn default() -> Self {
        Self::new()
    }
}

impl StfRunnerState {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for StfRunnerState {
    fn default() -> Self {
        Self::new()
    }
}

impl StfRunnerStatus {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for StfRunnerStatus {
    fn default() -> Self {
        Self::new()
    }
}
