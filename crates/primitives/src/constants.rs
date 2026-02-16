//! Constants for magic numbers and strings used in the primitives.

/// The size (in bytes) of a Hash (such as [`Txid`](bitcoin::Txid)).
pub const HASH_SIZE: usize = 32;

/// The size (in bytes) Execution environment Address.
pub const EE_ADDRESS_LEN: u8 = 20;

/// Number of blocks after bridge in transaction confirmation that the recovery path can be spent.
pub const RECOVER_DELAY: u16 = 1_008;

/// The number of timestamps used for calculating the median in Bitcoin header verification.
/// According to Bitcoin consensus rules, we need to check that a block's timestamp
/// is not lower than the median of the last eleven blocks' timestamps.
pub const TIMESTAMPS_FOR_MEDIAN: usize = 11;
