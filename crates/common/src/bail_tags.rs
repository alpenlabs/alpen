//! Bail tag identifiers used by debug crash injection.
//!
//! These constants are the single source of truth for the names of crash
//! injection bail points. They are referenced from the various
//! [`check_bail_trigger`](crate::check_bail_trigger) call sites and exposed
//! to functional tests via the `debug_listBailTags` RPC method (see
//! `bin/strata/src/rpc/mod.rs`).
//!
//! # Adding a new bail point
//!
//! 1. Define a `pub const BAIL_<NAME>: &str = "<name>";` in this file.
//! 2. Add it to [`KNOWN_BAIL_TAGS`].
//! 3. Use the constant at the `check_bail_trigger` call site, never a string literal.
//! 4. Functional tests pick up the new tag automatically via the `debug_listBailTags` RPC. No
//!    Python mirror to update.

/// Bail point in the OL sequencer's `handle_sign_block_duty` flow.
pub const BAIL_DUTY_SIGN_BLOCK: &str = "duty_sign_block";

/// Bail point in the fork-choice manager when processing a new L2 block.
pub const BAIL_FCM_NEW_BLOCK: &str = "fcm_new_block";

/// Bail point in the CSM worker when processing an ASM status event.
pub const BAIL_CSM_EVENT: &str = "csm_event";

/// Bail point in the CSM worker when finalizing an epoch.
pub const BAIL_CSM_EVENT_FINALIZE_EPOCH: &str = "csm_event_finalize_epoch";

/// All registered bail tags.
///
/// Returned to functional tests via the `debug_listBailTags` RPC so they can
/// validate tag strings without maintaining a Python-side mirror that can
/// drift from this list.
pub const KNOWN_BAIL_TAGS: &[&str] = &[
    BAIL_DUTY_SIGN_BLOCK,
    BAIL_FCM_NEW_BLOCK,
    BAIL_CSM_EVENT,
    BAIL_CSM_EVENT_FINALIZE_EPOCH,
];
