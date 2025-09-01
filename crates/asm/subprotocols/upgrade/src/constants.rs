/// The number of blocks during which an ASM verification key (VK) update remains in the
/// `QueuedUpdate` state and can still be cancelled by submitting a cancel transaction.
/// If no cancellation occurs within this many blocks of the original update transaction,
/// the upgrade transitions to `CommittedUpdate` and can no longer be cancelled.
pub(crate) const ASM_VK_QUEUE_DELAY: u64 = 12_960;

/// The number of blocks during which an optimistic L1 state-transition function (OL STF)
/// verification key (VK) update remains in the `QueuedUpdate` state and can still be
/// cancelled by submitting a cancel transaction. If no cancellation occurs within this
/// many blocks of the original update transaction, the update transitions to
/// `CommittedUpdate` and can no longer be cancelled.
pub(crate) const OL_STF_VK_QUEUE_DELAY: u64 = 4_320;

// ─── Enactment Delays ───────────────────────────────────────────────────────────

/// The number of blocks between when a multisig configuration update enters the
/// `ScheduledUpdate` state and when it is actually enacted. During this period, the
/// update is immutable and cannot be cancelled.
pub(crate) const MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY: u64 = 2_016;

/// The number of blocks between when an operator update enters the `ScheduledUpdate`
/// state and when it is actually enacted. During this period, the update is immutable
/// and cannot be cancelled.
pub(crate) const OPERATOR_UPDATE_ENACTMENT_DELAY: u64 = 2_016;

/// The number of blocks between when a sequencer update enters the `ScheduledUpdate`
/// state and when it is actually enacted. During this period, the update is immutable
/// and cannot be cancelled.
pub(crate) const SEQUENCER_UPDATE_ENACTMENT_DELAY: u64 = 2_016;

/// The number of blocks between when a verification key (VK) update enters the
/// `ScheduledUpdate` state and when it is actually enacted. During this period, the
/// update is immutable and cannot be cancelled.
pub(crate) const VK_UPDATE_ENACTMENT_DELAY: u64 = 144;
