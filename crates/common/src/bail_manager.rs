use std::{process, sync::LazyLock};

use tokio::sync::watch;
use tracing::*;

pub const BAIL_DUTY_SIGN_BLOCK: &str = "duty_sign_block";

struct BailWatch {
    sender: watch::Sender<Option<String>>,
    receiver: watch::Receiver<Option<String>>,
}

/// Singleton manager for `watch::Sender` and `watch::Receiver` used to communicate bail-out
/// contexts.
static BAIL_MANAGER: LazyLock<BailWatch> = LazyLock::new(|| {
    let (sender, receiver) = watch::channel(None);
    BailWatch { sender, receiver }
});

/// Publicly accessible `watch::Sender` for broadcasting bail-out context updates.
pub static BAIL_SENDER: LazyLock<watch::Sender<Option<String>>> =
    LazyLock::new(|| BAIL_MANAGER.sender.clone());

/// Publicly accessible `watch::Receiver` for subscribing to bail-out context updates.
pub static BAIL_RECEIVER: LazyLock<watch::Receiver<Option<String>>> =
    LazyLock::new(|| BAIL_MANAGER.receiver.clone());

/// Checks to see if we should bail out.
pub fn check_bail_trigger(ctx: &str) {
    if let Some(val) = BAIL_RECEIVER.borrow().clone() {
        warn!(%ctx, "tripped bail interrupt, exiting...");
        if ctx == val {
            process::exit(0);
        }
    }
}
