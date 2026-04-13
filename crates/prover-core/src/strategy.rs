//! Prove strategies: native and remote.
//!
//! Both are sync/blocking — called inside `spawn_blocking` by the prover.
//! The `Host` type is captured at build time and erased via `dyn ProveStrategy<H>`.

use std::sync::Arc;
#[cfg(feature = "remote")]
use std::time::Duration;

use zkaleido::{ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};

use crate::{
    error::{ProverError, ProverResult},
    spec::ProofSpec,
};

/// Context passed to [`ProveStrategy::prove`] for crash-recovery metadata.
///
/// Strategies that talk to remote provers (SP1, etc.) use this to:
/// 1. Check `saved` for a proof ID from a prior crashed run
/// 2. Call `persist()` right after `start_proving()` so the ID survives a crash
///
/// Strategies that don't need recovery (e.g. native) ignore this entirely.
pub struct ProveContext {
    /// Metadata from a prior run (e.g. serialized remote ProofId).
    pub saved: Option<Vec<u8>>,
    persist_fn: Option<Box<dyn FnOnce(Vec<u8>) + Send>>,
}

impl std::fmt::Debug for ProveContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProveContext")
            .field("saved", &self.saved.as_ref().map(|s| s.len()))
            .finish()
    }
}

impl ProveContext {
    pub fn new(
        saved: Option<Vec<u8>>,
        persist: impl FnOnce(Vec<u8>) + Send + 'static,
    ) -> Self {
        Self {
            saved,
            persist_fn: Some(Box::new(persist)),
        }
    }

    /// Persist metadata for crash recovery. Call this right after obtaining
    /// a remote proof ID, before starting the poll loop.
    pub fn persist(&mut self, data: Vec<u8>) {
        if let Some(f) = self.persist_fn.take() {
            f(data);
        }
    }

    /// Empty context — no saved metadata, persist is a no-op.
    pub fn empty() -> Self {
        Self {
            saved: None,
            persist_fn: None,
        }
    }
}

/// Blocking prove operation. Called inside `spawn_blocking`.
///
/// Implementations capture the zkVM host internally. The `Host` type
/// is erased when stored as `Arc<dyn ProveStrategy<H>>` in the prover.
pub trait ProveStrategy<H: ProofSpec>: Send + Sync + 'static {
    fn prove(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        ctx: ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata>;
}

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
        use tokio::{runtime::Builder, task::LocalSet, time::sleep};
        use zkaleido::RemoteProofStatus;

        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ProverError::Internal(e.into()))?;

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
        let _ = <H::Program as ZkVmProgram>::process_output::<Host>(
            receipt.receipt().public_values(),
        )
        .map_err(|e| ProverError::PermanentFailure(e.to_string()))?;

        Ok(receipt)
    }
}
