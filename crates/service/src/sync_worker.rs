//! Blocking worker task.

use tokio::sync::watch;
use tracing::*;

use crate::{Response, ServiceState, SyncService, SyncServiceInput};

pub(crate) fn worker_task<S: SyncService, I>(
    mut state: S::State,
    mut inp: I,
    status_tx: watch::Sender<S::Status>,
    shutdown_guard: strata_tasks::ShutdownGuard,
) -> anyhow::Result<()>
where
    I: SyncServiceInput<Msg = S::Msg>,
{
    let service = state.name().to_owned();

    // Perform startup logic.  If this errors we propagate it immediately and
    // crash the task.
    {
        let launch_span = debug_span!("onlaunch", %service);
        let _g = launch_span.enter();
        S::on_launch(&mut state)?;
    }

    // Process each message in a loop.  We do a shutdown check after each
    // possibly long-running call.
    let mut err = None;
    while let Some(input) = inp.recv_next()? {
        // Check after getting a new input.
        if shutdown_guard.should_shutdown() {
            debug!("got shutdown notification");
            break;
        }

        let input_span = debug_span!("handlemsg", %service, ?input);
        let _g = input_span.enter();

        // Process the input.
        let res = match S::process_input(&mut state, &input) {
            Ok(res) => res,
            Err(e) => {
                // TODO support optional retry
                error!(?input, %e, "failed to process message");
                err = Some(e);
                break;
            }
        };

        // Also check after processing input before trying to get a new one.
        if shutdown_guard.should_shutdown() {
            debug!("got shutdown notification");
            break;
        }

        // Update the status.
        let status = S::get_status(&state);
        let _ = status_tx.send(status);

        if res == Response::ShouldExit {
            break;
        }
    }

    // Perform shutdown handling.
    handle_shutdown::<S>(&mut state, err.as_ref());

    Ok(())
}

fn handle_shutdown<S: SyncService>(state: &mut S::State, err: Option<&anyhow::Error>) {
    if let Err(e) = S::before_shutdown(state, err) {
        error!(%e, "unhandled error while shutting down");
    }
}
