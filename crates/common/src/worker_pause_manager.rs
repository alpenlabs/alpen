use std::{collections::HashMap, sync::LazyLock, thread::sleep, time::Duration};

use tokio::sync::{mpsc, RwLock};
use tracing::*;

use crate::{Action, WorkerType};

/// Channel that receives signals from admin rpc to pause some of the internal workers.
struct PauseChannel {
    pub sender: mpsc::Sender<Action>,
    pub receiver: RwLock<mpsc::Receiver<Action>>,
}

impl PauseChannel {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel(10);
        Self {
            sender,
            receiver: RwLock::new(receiver),
        }
    }
}

#[derive(Debug)]
pub struct WorkerMessage {
    pub wtype: WorkerType,
    pub action: Action,
}

static PAUSE_CHANNELS: LazyLock<HashMap<WorkerType, PauseChannel>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert(WorkerType::SyncWorker, PauseChannel::new());
    m
});

/// Ask an worker to pause or resume it's work.
pub async fn send_action_to_worker(wtype: WorkerType, action: Action) -> bool {
    debug!(?wtype, ?action, "Received action for worker");
    let channel = PAUSE_CHANNELS.get(&wtype).unwrap();
    if let Err(e) = channel.sender.send(action).await {
        warn!(%e, "Could not send message to worker");
        return false;
    }
    true
}

/// For the given worker type, checks if it has a Pause message, if so pauses it.
pub async fn check_and_pause_debug_async(wtype: WorkerType) {
    let channel = PAUSE_CHANNELS.get(&wtype).unwrap();
    let mut receiver = channel.receiver.write().await;

    let should_wait = should_wait(&wtype, receiver.try_recv());

    if should_wait {
        loop {
            let should_resume = check_and_handle_action(&wtype, receiver.recv().await);
            if should_resume {
                break;
            }
        }
    }
}

/// For the given worker type, checks if it has a Pause message, if so pauses it.
pub fn check_and_pause_debug(wtype: WorkerType) {
    let channel = PAUSE_CHANNELS.get(&wtype).unwrap();
    let mut receiver = channel.receiver.blocking_write();

    let should_wait = should_wait(&wtype, receiver.try_recv());

    if should_wait {
        loop {
            let should_resume = check_and_handle_action(&wtype, receiver.blocking_recv());
            if should_resume {
                break;
            }
        }
    }
}

fn check_and_handle_action(wtype: &WorkerType, act: Option<Action>) -> bool {
    match act {
        Some(Action::Resume) => {
            debug!(?wtype, "Worker resuming");
            true
        }
        None => {
            debug!(?wtype, "Error receiving msg for worker");
            true
        }
        Some(m) => {
            debug!(?wtype, ?m, "Expecting Resume, got other");
            false
        }
    }
}

fn should_wait(wtype: &WorkerType, d: Result<Action, mpsc::error::TryRecvError>) -> bool {
    match d {
        Ok(Action::Pause(secs)) => {
            debug!(?wtype, %secs, "Worker pausing");
            sleep(Duration::from_secs(secs));
            false
        }
        Ok(Action::PauseUntilResume) => {
            debug!(?wtype, "Worker pausing indefinitely");
            true
        }
        Ok(Action::Resume) => {
            debug!(?wtype, "Worker resuming");
            false
        }
        _ => {
            // Just return
            false
        }
    }
}
