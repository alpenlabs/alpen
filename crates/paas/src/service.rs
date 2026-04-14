//! Service Framework integration.
//!
//! Two modes:
//! - **Command-only**: no tick, no retries. Good for one-shot provers.
//! - **Ticking**: periodic `prover.tick()` for retry scanning and startup recovery.

use std::{fmt, marker::PhantomData, sync::Arc, time::Duration};

use strata_prover_core::{ProofSpec, Prover, ProverResult, TaskResult};
use strata_service::{
    AsyncService, CommandCompletionSender, CommandHandle, Response, Service, ServiceBuilder,
    ServiceState, TickMsg, TickingInput, TokioMpscInput,
};
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;
use tracing::{info, trace};

use crate::handle::ProverHandle;

// ============================================================================
// Commands
// ============================================================================

#[derive(Debug)]
pub(crate) enum Cmd<T: Clone + fmt::Debug + Send + Sync + 'static> {
    Submit {
        task: T,
        completion: CommandCompletionSender<ProverResult<()>>,
    },
    Execute {
        task: T,
        completion: CommandCompletionSender<ProverResult<TaskResult<T>>>,
    },
}

// ============================================================================
// Shared state
// ============================================================================

pub(crate) struct State<H: ProofSpec> {
    pub(crate) prover: Arc<Prover<H>>,
}

impl<H: ProofSpec> fmt::Debug for State<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State").finish()
    }
}

impl<H: ProofSpec> Clone for State<H> {
    fn clone(&self) -> Self {
        Self {
            prover: self.prover.clone(),
        }
    }
}

impl<H: ProofSpec> ServiceState for State<H> {
    fn name(&self) -> &str {
        "prover"
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProverServiceStatus {
    pub task_count: usize,
}

// ============================================================================
// Command handling
// ============================================================================

async fn handle_cmd<H: ProofSpec>(prover: &Prover<H>, cmd: Cmd<H::Task>) {
    match cmd {
        Cmd::Submit { task, completion } => {
            trace!(?task, "handling submit command");
            let result = prover.submit(task).await;
            completion.send(result).await;
        }
        Cmd::Execute { task, completion } => {
            trace!(?task, "handling execute command");
            let result = prover.execute(task).await;
            completion.send(result).await;
        }
    }
}

// ============================================================================
// Mode 1: Commands only (no tick)
// ============================================================================

pub(crate) struct CmdOnlySvc<H: ProofSpec>(PhantomData<H>);

impl<H: ProofSpec> fmt::Debug for CmdOnlySvc<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProverService").finish()
    }
}

impl<H: ProofSpec> Service for CmdOnlySvc<H> {
    type State = State<H>;
    type Msg = Cmd<H::Task>;
    type Status = ProverServiceStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        ProverServiceStatus {
            task_count: state.prover.task_store().count().unwrap_or(0),
        }
    }
}

impl<H: ProofSpec> AsyncService for CmdOnlySvc<H> {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        info!("prover service launched (command-only)");
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        handle_cmd(&state.prover, input).await;
        Ok(Response::Continue)
    }
}

// ============================================================================
// Mode 2: Commands + Tick (retries, recovery)
// ============================================================================

pub(crate) struct TickingSvc<H: ProofSpec>(PhantomData<H>);

impl<H: ProofSpec> fmt::Debug for TickingSvc<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProverService(ticking)").finish()
    }
}

impl<H: ProofSpec> Service for TickingSvc<H> {
    type State = State<H>;
    type Msg = TickMsg<Cmd<H::Task>>;
    type Status = ProverServiceStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        ProverServiceStatus {
            task_count: state.prover.task_store().count().unwrap_or(0),
        }
    }
}

impl<H: ProofSpec> AsyncService for TickingSvc<H> {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        info!("prover service launched (ticking)");
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match input {
            TickMsg::Msg(cmd) => handle_cmd(&state.prover, cmd).await,
            TickMsg::Tick => {
                trace!("tick: scanning for retriable tasks");
                state.prover.tick().await;
            }
        }
        Ok(Response::Continue)
    }
}

// ============================================================================
// Builder
// ============================================================================

/// Launches a [`Prover`] as a Service Framework service.
pub struct ProverServiceBuilder<H: ProofSpec> {
    prover: Prover<H>,
    tick_interval: Option<Duration>,
}

impl<H: ProofSpec> ProverServiceBuilder<H> {
    pub fn new(prover: Prover<H>) -> Self {
        Self {
            prover,
            tick_interval: None,
        }
    }

    /// Enable tick-based retry scanning and startup recovery.
    pub fn tick_interval(mut self, interval: Duration) -> Self {
        self.tick_interval = Some(interval);
        self
    }

    /// Launch the service, returning a [`ProverHandle`] for consumer use.
    ///
    /// Fails fast if the prover is configured with a retry policy but no
    /// `tick_interval` — in that configuration retries would silently never
    /// fire, which is almost certainly a misconfiguration.
    pub async fn launch(self, executor: &TaskExecutor) -> anyhow::Result<ProverHandle<H>> {
        if self.prover.has_retry() && self.tick_interval.is_none() {
            anyhow::bail!(
                "ProverServiceBuilder: retry is configured but tick_interval is unset; \
                 retries need a tick to fire — call .tick_interval(..) or drop the retry config"
            );
        }

        let prover = Arc::new(self.prover);
        let state = State {
            prover: prover.clone(),
        };

        if let Some(interval) = self.tick_interval {
            let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd<H::Task>>(100);
            let cmd_handle = Arc::new(CommandHandle::new(cmd_tx));
            let cmd_input = TokioMpscInput::new(cmd_rx);
            let ticking = TickingInput::new(interval, cmd_input);

            let builder = ServiceBuilder::<TickingSvc<H>, _>::new()
                .with_state(state)
                .with_input(ticking);
            let monitor = builder.launch_async("prover", executor).await?;

            Ok(ProverHandle::new(cmd_handle, monitor, prover))
        } else {
            let mut builder = ServiceBuilder::<CmdOnlySvc<H>, _>::new().with_state(state);
            let cmd_handle = Arc::new(builder.create_command_handle(100));
            let monitor = builder.launch_async("prover", executor).await?;

            Ok(ProverHandle::new(cmd_handle, monitor, prover))
        }
    }
}

impl<H: ProofSpec> fmt::Debug for ProverServiceBuilder<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProverServiceBuilder").finish()
    }
}
