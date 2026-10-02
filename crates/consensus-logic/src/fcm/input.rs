use std::{fmt, time::Duration};

use strata_csm_types::CheckpointState;
use strata_service::{
    AsyncServiceInput, Either, SelectInput, ServiceInput, TickMsg, TickingInput, TokioMpscInput,
    TokioWatchInput,
};
use tokio::sync::{mpsc, watch};
use tracing::trace;

use crate::message::ForkChoiceMessage;

#[derive(Clone, Debug)]
pub enum FcmEvent {
    NewFcmMsg(ForkChoiceMessage),
    NewStateUpdate,
    RetryTick,
    Abort,
}

type FcmSources = SelectInput<TokioMpscInput<ForkChoiceMessage>, TokioWatchInput<CheckpointState>>;

/// Maps command, checkpoint, and timer inputs to FCM events.
///
/// Closing either channel aborts FCM. The shared ticking adapter skips missed ticks
/// so a slow retry pass does not produce a burst of catch-up retries.
pub struct FcmInput {
    inner: TickingInput<FcmSources>,
}

impl fmt::Debug for FcmInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FcmInput").finish_non_exhaustive()
    }
}

impl FcmInput {
    pub fn new(
        fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
        checkpoint_state_rx: watch::Receiver<CheckpointState>,
    ) -> Self {
        let sources = SelectInput::new(
            TokioMpscInput::new(fcm_rx),
            TokioWatchInput::from_receiver(checkpoint_state_rx),
        );
        Self {
            inner: TickingInput::new(Duration::from_secs(1), sources),
        }
    }
}

impl ServiceInput for FcmInput {
    type Msg = FcmEvent;
}

impl AsyncServiceInput for FcmInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        let event = match self.inner.recv_next().await? {
            Some(TickMsg::Tick) => FcmEvent::RetryTick,
            Some(TickMsg::Msg(Either::Left(message))) => FcmEvent::NewFcmMsg(message),
            Some(TickMsg::Msg(Either::Right(_))) => FcmEvent::NewStateUpdate,
            None => {
                trace!("FCM input channel closed");
                FcmEvent::Abort
            }
        };
        Ok(Some(event))
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::OLBlockId;
    use tokio::time::timeout;

    use super::*;

    async fn next_event(input: &mut FcmInput) -> FcmEvent {
        timeout(Duration::from_millis(500), input.recv_next())
            .await
            .expect("FCM input should be ready")
            .expect("FCM input should succeed")
            .expect("FCM input should produce an event")
    }

    #[tokio::test]
    async fn forwards_commands_and_checkpoint_changes_after_retry_tick() {
        let (commands, command_rx) = mpsc::channel(1);
        let (checkpoints, checkpoint_rx) = watch::channel(CheckpointState::default());
        let mut input = FcmInput::new(command_rx, checkpoint_rx);

        assert!(matches!(next_event(&mut input).await, FcmEvent::RetryTick));

        let id = OLBlockId::null();
        commands
            .send(ForkChoiceMessage::NewBlock(id))
            .await
            .unwrap();
        assert!(matches!(
            next_event(&mut input).await,
            FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(received)) if received == id
        ));

        checkpoints.send_modify(|_| {});
        assert!(matches!(
            next_event(&mut input).await,
            FcmEvent::NewStateUpdate
        ));
    }

    #[tokio::test]
    async fn aborts_when_command_channel_closes() {
        let (commands, command_rx) = mpsc::channel(1);
        let (_checkpoints, checkpoint_rx) = watch::channel(CheckpointState::default());
        let mut input = FcmInput::new(command_rx, checkpoint_rx);
        assert!(matches!(next_event(&mut input).await, FcmEvent::RetryTick));

        drop(commands);
        assert!(matches!(next_event(&mut input).await, FcmEvent::Abort));
    }

    #[tokio::test]
    async fn aborts_when_checkpoint_channel_closes() {
        let (_commands, command_rx) = mpsc::channel(1);
        let (checkpoints, checkpoint_rx) = watch::channel(CheckpointState::default());
        let mut input = FcmInput::new(command_rx, checkpoint_rx);
        assert!(matches!(next_event(&mut input).await, FcmEvent::RetryTick));

        drop(checkpoints);
        assert!(matches!(next_event(&mut input).await, FcmEvent::Abort));
    }
}
