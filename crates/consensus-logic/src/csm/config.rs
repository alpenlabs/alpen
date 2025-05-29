use std::time;

const RETRY_BACKOFF_BASE: u32 = 1024;

/// Run-time config for CSM executor.
///
/// This is *not* like system params.
#[derive(Debug)]
pub struct CsmExecConfig {
    /// Base retry duration, which is increases exponentially for each retry.
    pub retry_base_dur: time::Duration,

    /// Maximum retry count.
    pub retry_cnt_max: u32,

    /// Retry backoff multiplier used to control the exponential backoff.
    ///
    /// This is multiplied against the current wait dur and then divided by
    /// 1024.  A sensible value for this should ensure that we don't sleep more
    /// than 10x-20x `retry_base_dur` before terminating.
    pub retry_backoff_mult: u32,
}

impl CsmExecConfig {
    /// Computes the next step of retry backoff.  This is effectively a fixp
    /// multiplication by the `retry_backoff_mult`.
    pub fn compute_retry_backoff(&self, cur_dur: time::Duration) -> time::Duration {
        (cur_dur * self.retry_backoff_mult) / RETRY_BACKOFF_BASE
    }
}
