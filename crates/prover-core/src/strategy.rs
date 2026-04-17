//! Concrete [`ProveStrategy`](crate::ProveStrategy) impls: native and remote.
//!
//! Both are sync/blocking — called inside `spawn_blocking` by the prover.
//! The `Host` type is captured at build time and erased via
//! `dyn ProveStrategy<H>`.

use std::sync::Arc;
#[cfg(feature = "remote")]
use std::time::Duration;

use zkaleido::{ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};

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
        H::Program::prove(input, self.host.as_ref())
            .map_err(|e| ProverError::PermanentFailure(e.to_string()))
    }
}

/// Remote execution: `start_proving` + poll + `get_proof` via a `LocalSet`.
///
/// `ZkVmRemoteProver` returns `!Send` futures, so we spin up a current-thread
/// runtime with `LocalSet` inside `spawn_blocking` to contain them.
///
/// On crash recovery, if `ctx.saved` contains a serialized `ProofId`, we skip
/// `start_proving` and resume polling directly — no double submission.
#[cfg(feature = "remote")]
pub(crate) struct RemoteStrategy<Host> {
    host: Arc<Host>,
    poll_interval: Duration,
}

#[cfg(feature = "remote")]
impl<Host> RemoteStrategy<Host> {
    pub(crate) fn new(host: Host, poll_interval: Duration) -> Self {
        Self {
            host: Arc::new(host),
            poll_interval,
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
        use tokio::{runtime::Builder, task::LocalSet};

        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ProverError::Storage(format!("tokio runtime: {e}")))?;

        let local = LocalSet::new();
        let host = self.host.clone();
        let poll_interval = self.poll_interval;

        local.block_on(&rt, async move {
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
            .map_err(|e| ProverError::PermanentFailure(e.to_string()))?;

        let proof_id = host
            .start_proving(prepared, H::Program::proof_type())
            .await
            .map_err(|e| ProverError::TransientFailure(e.to_string()))?;

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
        use tokio::time::sleep;
        use zkaleido::RemoteProofStatus;

        loop {
            let status = host
                .get_status(proof_id)
                .await
                .map_err(|e| ProverError::TransientFailure(e.to_string()))?;

            match status {
                RemoteProofStatus::Completed => {
                    tracing::info!(%proof_id, "remote proof completed");
                    break;
                }
                RemoteProofStatus::Failed(reason) => {
                    return Err(ProverError::PermanentFailure(format!(
                        "remote proof failed: {reason}"
                    )));
                }
                RemoteProofStatus::Requested | RemoteProofStatus::InProgress => {
                    sleep(poll_interval).await;
                }
                RemoteProofStatus::Unknown => {
                    tracing::warn!(%proof_id, "unknown remote proof status, retrying");
                    sleep(poll_interval).await;
                }
            }
        }

        // Retrieve the receipt.
        let receipt = host
            .get_proof(proof_id)
            .await
            .map_err(|e| ProverError::PermanentFailure(e.to_string()))?;

        // Verify output is well-formed.
        let _ =
            <H::Program as ZkVmProgram>::process_output::<Host>(receipt.receipt().public_values())
                .map_err(|e| ProverError::PermanentFailure(e.to_string()))?;

        Ok(receipt)
    }
}
