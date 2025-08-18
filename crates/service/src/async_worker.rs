//! Async service worker task.

use futures::FutureExt;
use tokio::sync::watch;
use tracing::*;

use crate::{AsyncService, AsyncServiceInput, Response, ServiceState};

/// Async worker task.
pub(crate) async fn worker_task<S: AsyncService, I>(
    mut state: S::State,
    mut inp: I,
    status_tx: watch::Sender<S::Status>,
    shutdown_guard: strata_tasks::ShutdownGuard,
) -> anyhow::Result<()>
where
    I: AsyncServiceInput<Msg = S::Msg>,
{
    // Perform startup logic.  If this errors we propagate it immediately and
    // crash the task.
    {
        let service = state.name().to_owned();
        let launch_span = debug_span!("onlaunch", %service);
        S::on_launch(&mut state).instrument(launch_span).await?;
    }

    // Wrapping for the worker task to respect shutdown requests.
    let err = {
        let mut exit_fut = Box::pin(shutdown_guard.wait_for_shutdown().fuse());
        let mut wkr_fut =
            Box::pin(worker_task_inner::<S, I>(&mut state, &mut inp, &status_tx).fuse());

        futures::select! {
            _ = exit_fut => None,
            res = wkr_fut => res.err(),
        }
    };

    // Perform shutdown handling.
    handle_shutdown::<S>(&mut state, err.as_ref()).await;

    Ok(())
}

async fn worker_task_inner<S: AsyncService, I>(
    state: &mut S::State,
    inp: &mut I,
    status_tx: &watch::Sender<S::Status>,
) -> anyhow::Result<()>
where
    I: AsyncServiceInput<Msg = S::Msg>,
{
    let service = state.name().to_owned();

    // This is preliminary, we'll make it more sophisticated in the future.
    while let Some(input) = inp.recv_next().await? {
        let input_span = debug_span!("handlemsg", %service, ?input);

        // Process the input.
        let res = match S::process_input(state, &input).instrument(input_span).await {
            Ok(res) => res,
            Err(e) => {
                // TODO support optional retry
                error!(?input, %e, "failed to process message");
                return Err(e);
            }
        };

        // Update the status.
        let status = S::get_status(state);
        let _ = status_tx.send(status);

        if res == Response::ShouldExit {
            break;
        }
    }

    Ok(())
}

async fn handle_shutdown<S: AsyncService>(state: &mut S::State, err: Option<&anyhow::Error>) {
    if let Err(e) = S::before_shutdown(state, err).await {
        error!(%e, "unhandled error while shutting down");
    }
}
