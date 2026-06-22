//! Concrete [`ProveStrategy`](crate::ProveStrategy) impls: native and remote.
//!
//! Both are sync/blocking — called inside `spawn_blocking` by the prover.
//! The `Host` type is captured at build time and erased via
//! `dyn ProveStrategy<H>`.

use std::sync::Arc;
#[cfg(feature = "remote")]
use std::{future::Future, time::Duration};

#[cfg(feature = "remote")]
use tokio::{
    runtime::{Builder, Runtime},
    time::sleep,
};
use zkaleido::{ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};

#[cfg(feature = "remote")]
use crate::config::LocalRetryConfig;
use crate::{
    error::{ProverError, ProverResult},
    traits::{ProofSpec, ProveContext, ProveStrategy},
};

/// Native execution: `ZkVmProgram::prove` directly.
pub(crate) struct NativeStrategy<Host> {
    host: Arc<Host>,
}

impl<Host> NativeStrategy<Host> {
    pub(crate) fn new(host: Host) -> Self {
        Self {
            host: Arc::new(host),
        }
    }
}

impl<H, Host> ProveStrategy<H> for NativeStrategy<Host>
where
    H: ProofSpec,
    Host: ZkVmHost + Send + Sync + 'static,
{
    fn prove(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        _ctx: ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        H::Program::prove(input, self.host.as_ref()).map_err(ProverError::from_zkvm)
    }
}

/// Remote execution: `start_proving` + poll + `get_proof`.
///
/// Each `prove()` drives the remote futures on a **single long-lived runtime**
/// owned by the strategy, not a fresh runtime per call. The futures are `Send`
/// (zkaleido v0.3.0-rc.1+ defines `ZkVmRemoteProgram::start_proving` as
/// `-> impl Future + Send`), so they run directly on the multi-thread runtime —
/// no `LocalSet` thread-pinning needed.
///
/// SP1 SDK >=6.2 caches its gRPC channel for the whole process, and a tonic
/// channel is just an mpsc handle to a background worker `tokio::spawn`'d on the
/// runtime that was active when the channel was first built. A per-call runtime
/// would be dropped after the first prove, killing that worker and leaving the
/// cached channel permanently unusable — every later call would then fail with
/// "Service was not ready: transport error". The shared runtime keeps the worker
/// alive for as long as the strategy exists (i.e. the process).
///
/// On crash recovery, if `ctx.saved` contains a serialized `ProofId`, we skip
/// `start_proving` and resume polling directly — no double submission.
#[cfg(feature = "remote")]
pub(crate) struct RemoteStrategy<Host> {
    host: Arc<Host>,
    poll_interval: Duration,
    /// Long-lived runtime shared across every `prove()` call so the SP1 gRPC
    /// channel's background worker (spawned on first use) outlives individual
    /// proves. See the type-level docs for why this must not be per-call.
    ///
    /// `Option` only so [`Drop`] can take it and shut it down without blocking;
    /// it is `Some` for the entire normal lifetime.
    rt: Option<Runtime>,
    /// In-attempt retry budget for the idempotent remote polls.
    local_retry: LocalRetryConfig,
}

#[cfg(feature = "remote")]
impl<Host> Drop for RemoteStrategy<Host> {
    fn drop(&mut self) {
        // The default `Runtime` drop performs a *blocking* shutdown, which
        // panics if it runs inside an async context — and these strategies are
        // dropped during async service teardown (the bins build remote provers
        // inside the tokio runtime). `shutdown_background` releases the runtime
        // without blocking, so it is safe to drop from any context.
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

#[cfg(feature = "remote")]
impl<Host> RemoteStrategy<Host> {
    pub(crate) fn new(host: Host, poll_interval: Duration, local_retry: LocalRetryConfig) -> Self {
        // Multi-thread (not current-thread): concurrent fanned-out `prove()`
        // calls each run on their own `spawn_blocking` thread and `block_on`
        // this runtime simultaneously; a current-thread runtime would serialize
        // them. The remote prove future itself runs on the calling thread via
        // `Runtime::block_on`; these worker threads only drive the shared gRPC
        // channel and its IO, so a small pool is plenty.
        let rt = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("paas-remote-prover")
            .enable_all()
            .build()
            .expect("build remote-prover runtime");
        Self {
            host: Arc::new(host),
            poll_interval,
            rt: Some(rt),
            local_retry,
        }
    }
}

/// Retry an idempotent backend op in-process on transient (`RetryResume`)
/// errors, with short backoff, before surfacing to the task-level tier.
///
/// Complements the backend's own transport retry: SP1 marks some transient
/// transport failures (e.g. "Service was not ready") as permanent and gives up
/// quickly, so a brief local re-poll recovers them without a full task restart.
/// Permanent errors are returned immediately.
#[cfg(feature = "remote")]
async fn with_local_retry<T, F, Fut>(
    cfg: &LocalRetryConfig,
    op_name: &str,
    mut op: F,
) -> ProverResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ProverResult<T>>,
{
    use crate::error::FailureAction;

    let mut attempt: u32 = 0;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                attempt += 1;
                if attempt > cfg.max_attempts || e.action() != FailureAction::RetryResume {
                    return Err(e);
                }
                tracing::warn!(op = op_name, attempt, error = %e, "in-attempt retry");
                sleep(cfg.delay(attempt)).await;
            }
        }
    }
}

#[cfg(feature = "remote")]
impl<H, Host> ProveStrategy<H> for RemoteStrategy<Host>
where
    H: ProofSpec,
    Host: zkaleido::ZkVmRemoteHost + Send + Sync + 'static,
{
    fn prove(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        mut ctx: ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        let host = self.host.clone();
        let poll_interval = self.poll_interval;

        // Drive on the strategy's long-lived runtime (see the type-level docs):
        // a fresh per-call runtime would kill SP1's process-cached gRPC channel
        // worker after the first prove. `rt` is `Some` for the whole lifetime;
        // it is only taken in `Drop`.
        let rt = self
            .rt
            .as_ref()
            .expect("remote-prover runtime present outside Drop");
        rt.block_on(async move {
            // Try to resume from saved metadata (prior crash).
            let proof_id = if let Some(saved) = ctx.saved.take() {
                match Host::ProofId::try_from(saved) {
                    Ok(id) => {
                        tracing::info!(%id, "resuming remote proof from saved metadata");
                        id
                    }
                    Err(_) => {
                        tracing::warn!("failed to deserialize saved ProofId, starting fresh");
                        return self.start_fresh::<H>(input, &host, &mut ctx).await;
                    }
                }
            } else {
                // Fresh submission.
                let id = self.submit_proof::<H>(input, &host).await?;
                ctx.persist(id.clone().into());
                id
            };

            // Poll until completion.
            self.poll_until_done::<H>(&host, &proof_id, poll_interval)
                .await
        })
    }
}

#[cfg(feature = "remote")]
impl<Host> RemoteStrategy<Host>
where
    Host: zkaleido::ZkVmRemoteHost + Send + Sync + 'static,
{
    async fn submit_proof<H: ProofSpec>(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        host: &Host,
    ) -> ProverResult<Host::ProofId> {
        let prepared = <H::Program as ZkVmProgram>::prepare_input::<Host::Input<'_>>(input)
            .map_err(|e| ProverError::from_zkvm(e.into()))?;

        let proof_id = host
            .start_proving(prepared, H::Program::proof_type())
            .await
            .map_err(ProverError::from_zkvm)?;

        tracing::info!(%proof_id, "remote proof submitted");
        Ok(proof_id)
    }

    async fn start_fresh<H: ProofSpec>(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        host: &Host,
        ctx: &mut ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        let proof_id = self.submit_proof::<H>(input, host).await?;
        ctx.persist(proof_id.clone().into());
        self.poll_until_done::<H>(host, &proof_id, self.poll_interval)
            .await
    }

    async fn poll_until_done<H: ProofSpec>(
        &self,
        host: &Host,
        proof_id: &Host::ProofId,
        poll_interval: Duration,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        use zkaleido::RemoteProofStatus;

        use crate::classify::classify_remote_failure;

        loop {
            let status = with_local_retry(&self.local_retry, "get_status", || async {
                host.get_status(proof_id)
                    .await
                    .map_err(ProverError::from_zkvm)
            })
            .await?;

            match status {
                RemoteProofStatus::Completed => {
                    tracing::info!(%proof_id, "remote proof completed");
                    break;
                }
                RemoteProofStatus::Failed(reason) => {
                    return Err(ProverError::Failed {
                        action: classify_remote_failure(&reason),
                        msg: format!("remote proof failed: {reason}"),
                    });
                }
                RemoteProofStatus::Requested | RemoteProofStatus::InProgress => {
                    sleep(poll_interval).await;
                }
            }
        }

        // Retrieve the receipt.
        let receipt = with_local_retry(&self.local_retry, "get_proof", || async {
            host.get_proof(proof_id)
                .await
                .map_err(ProverError::from_zkvm)
        })
        .await?;

        // Verify output is well-formed.
        let _ =
            <H::Program as ZkVmProgram>::process_output::<Host>(receipt.receipt().public_values())
                .map_err(ProverError::from_zkvm)?;

        Ok(receipt)
    }
}
