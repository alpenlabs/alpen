//! Predicate key (update VK) rotation message type.

/// Message type ID for snark-account predicate key (update VK) rotations.
///
/// Emitted by the OL STF into the target account's inbox when an admin
/// predicate update is applied, so the execution environment observes the
/// pending rotation at a deterministic position in its inbox ordering. The
/// OL attaches no semantics to its consumption — rotations only activate
/// through the update's own declared predicate. Per the Alpen upgrade
/// design, the EE's policy is to terminate the batch that consumes this
/// message and declare the queued key, making that batch the last one
/// proven under the old VK.
///
/// The message body is the SSZ encoding of the new `PredicateKey` (defined
/// in `strata-predicate`, which this crate deliberately does not depend on).
pub const PREDICATE_UPDATE_MSG_TYPE_ID: u16 = 0x20;
