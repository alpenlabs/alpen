//! Async service worker task.

use tokio::sync::watch;
use tracing::*;

use super::*;

/// Async worker task.
pub async fn worker_task<S: AsyncService>(
    mut state: S::State,
    mut inp: S::Input,
    status_tx: watch::Sender<S::Status>,
) -> anyhow::Result<()>
where
    S::Input: AsyncServiceInput,
{
    let service = state.name().to_owned();

    // This is preliminary, we'll make it more sophisticated in the future.
    while let Some(input) = inp.recv_next().await? {
        let input_span = debug_span!("handlemsg", %service, ?input);

        // Process the input.
        let res = match S::process_input(&mut state, &input)
            .instrument(input_span)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                // TODO support optional retry
                error!(?input, %e, "failed to process message");
                break;
            }
        };

        // Update the status.
        let status = S::get_status(&state);
        let _ = status_tx.send(status);

        if res == Response::ShouldExit {
            break;
        }
    }

    Ok(())
}
