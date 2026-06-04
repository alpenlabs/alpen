//! Handle for whichever OL sync service the node runs.

use std::sync::Arc;

use crate::{checkpoint_sync::CssServiceHandle, FcmServiceHandle};

/// A node runs exactly one OL sync service: the fork-choice manager when it is
/// a sequencer, the checkpoint sync service otherwise. The `Css` variant is
/// held only to keep the service alive.
#[derive(Debug)]
pub enum SyncServiceHandle {
    /// Fork-choice manager handle (sequencer nodes).
    Fcm(Arc<FcmServiceHandle>),
    /// Checkpoint sync service handle (non-sequencer nodes).
    Css(Arc<CssServiceHandle>),
}
