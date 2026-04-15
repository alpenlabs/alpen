//! Replay pipeline types.

use std::fmt::{self, Display};

use serde::Serialize;

/// Starting state used for replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayStart {
    Genesis,
    Snapshot,
}

impl Display for ReplayStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Genesis => f.write_str("genesis"),
            Self::Snapshot => f.write_str("snapshot"),
        }
    }
}
