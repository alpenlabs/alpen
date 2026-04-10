//! Input types for the signer service.
//!
//! Uses the framework's [`TickingInput`] + [`TokioMpscInput`] combo so that
//! the service wakes up on a periodic timer *or* when a duty-resolution
//! notification arrives, without any hand-rolled fan-in logic.

use strata_service::TickMsg;

use crate::service::DutyResolved;

/// Message type for the signer service.
///
/// Either a regular poll tick (time to fetch duties) or a duty-resolution
/// notification (success or failure) carrying the duty ID to evict from the seen-set.
pub(crate) type SignerMsg = TickMsg<DutyResolved>;
