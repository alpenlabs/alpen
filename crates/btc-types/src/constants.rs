/// The number of timestamps used for calculating the median in Bitcoin header verification.
/// According to Bitcoin consensus rules, we need to check that a block's timestamp
/// is not lower than the median of the last eleven blocks' timestamps.
pub const TIMESTAMPS_FOR_MEDIAN: usize = 11;

/// The size (in bytes) of a Hash (such as [`Txid`](bitcoin::Txid)).
pub const HASH_SIZE: usize = 32;
