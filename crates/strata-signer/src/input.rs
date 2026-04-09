//! Input types for the signer service.
//!
//! Uses the framework's [`TickingInput`] + [`TokioMpscInput`] combo so that
//! the service wakes up on a periodic timer *or* when a duty-failure
//! notification arrives, without any hand-rolled fan-in logic.

use strata_primitives::buf::Buf32;
use strata_service::TickMsg;

/// Message type for the signer service.
///
/// Either a regular poll tick (time to fetch duties) or a duty-failure
/// notification carrying the duty ID to evict from the seen-set for retry.
pub(crate) type SignerMsg = TickMsg<Buf32>;
