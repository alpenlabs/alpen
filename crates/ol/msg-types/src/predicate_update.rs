//! Predicate key (update VK) rotation message type.

/// Message type ID for snark-account predicate key (update VK) rotations.
///
/// Emitted by the OL STF into the target account's inbox when an admin
/// predicate update is applied, so the execution environment observes the
/// rotation at a deterministic position in its inbox ordering. Per the Alpen
/// upgrade design, this message is the consensus-level fork boundary: the EE
/// batch that consumes it is the last one proven under the old VK.
///
/// The message body is the SSZ encoding of the new
/// [`PredicateKey`](strata_predicate::PredicateKey).
pub const PREDICATE_UPDATE_MSG_TYPE_ID: u16 = 0x20;
