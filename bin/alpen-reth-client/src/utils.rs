use std::time;

pub trait BackoffStrategy {
    fn next_duration(&self, current: u64, count: u64) -> u64;
}

#[derive(Debug, Clone)]
pub struct RetryTracker<BackoffStrategy> {
    base_duration: u64,
    max_duration: u64,
    current_duration: u64,
    retry_count: u64,
    backoff: BackoffStrategy,
}

impl<B: BackoffStrategy> RetryTracker<B> {
    pub fn new(base_duration: u64, max_duration: u64, backoff: B) -> Self {
        Self {
            base_duration,
            max_duration,
            current_duration: base_duration,
            retry_count: 0,
            backoff,
        }
    }

    pub fn reset(&mut self) {
        self.retry_count = 0;
        self.current_duration = self.base_duration;
    }

    pub fn increment(&mut self) {
        self.retry_count += 1;
        self.current_duration = self
            .backoff
            .next_duration(self.current_duration, self.retry_count)
            .min(self.max_duration);
    }

    pub fn delay(&self) -> u64 {
        self.current_duration
    }

    pub fn count(&self) -> u64 {
        self.retry_count
    }
}

pub struct ExponentialBackoff {
    multiplier: f64,
}

impl ExponentialBackoff {
    pub fn new(multiplier: f64) -> Self {
        Self { multiplier }
    }
}

impl BackoffStrategy for ExponentialBackoff {
    fn next_duration(&self, current: u64, _count: u64) -> u64 {
        (current as f64 * self.multiplier) as u64
    }
}

pub trait ClockProvider {
    fn now_millis(&self) -> u64;
}

pub struct SystemClock;

impl ClockProvider for SystemClock {
    fn now_millis(&self) -> u64 {
        time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64
    }
}
