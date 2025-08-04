use std::{thread::sleep, time::Duration};

use tracing::{error, warn};

pub mod policies;

/// Default maximum number of retries for engine calls.
pub const DEFAULT_ENGINE_CALL_MAX_RETRIES: u16 = 4;

/// Runs a fallible operation with a backoff retry.
///
/// Retries the given `operation` up to `max_retries` times with delays
/// increasing according to the provided config that implements [`Backoff`] trait.
/// The operation will be executed at most `max_retries` times total.
///
/// Logs a warning on each failure and an error if all retries are exhausted.
///
/// # Parameters
///
/// - `name`: Identifier used in logs for the operation.
/// - `max_retries`: Maximum number of retry attempts (total executions will be max_retries).
/// - `backoff`: Backoff configuration for computing delay.
/// - `operation`: Closure returning `Result`; retried on `Err`.
///
/// # Returns
///
/// - `Ok(R)` if the operation succeeds within allowed attempts.
/// - `Err(E)` if all attempts fail.
///
/// # Example
///
/// ```rust
/// use strata_common::retry::{policies::ExponentialBackoff, retry_with_backoff};
///
/// // A dummy function for the example.
/// fn try_something() -> Result<(), &'static str> {
///     // In a real scenario, this would attempt an operation that might fail.
///     Err("failed to do something")
/// }
///
/// let result = retry_with_backoff(
///     "my_task",
///     3,
///     &ExponentialBackoff::new(500, 150, 100),
///     || try_something(),
/// );
/// ```
pub fn retry_with_backoff<R, E, F>(
    name: &str,
    max_retries: u16,
    backoff: &impl Backoff,
    operation: F,
) -> Result<R, E>
where
    F: Fn() -> Result<R, E>,
    E: std::fmt::Debug,
{
    retry_with_backoff_inner(name, max_retries, backoff, operation, sleep)
}

/// Inner method that actually does the retry which is generic on the sleep function.
fn retry_with_backoff_inner<R, E, F, S>(
    name: &str,
    max_retries: u16,
    backoff: &impl Backoff,
    operation: F,
    mut sleep_fn: S,
) -> Result<R, E>
where
    F: Fn() -> Result<R, E>,
    E: std::fmt::Debug,
    S: FnMut(Duration),
{
    let mut delay = backoff.base_delay_ms();

    for attempt in 0..max_retries {
        match operation() {
            Ok(value) => return Ok(value),
            Err(err) if attempt < max_retries - 1 => {
                warn!(
                    "Attempt {} failed with {:?} while running {}. Retrying in {:?}",
                    attempt + 1,
                    err,
                    name,
                    delay
                );
                sleep_fn(Duration::from_millis(delay));
                delay = backoff.next_delay_ms(delay);
            }
            Err(err) => {
                error!(
                    "Max retries exceeded while running {}, returning with the last error",
                    name
                );
                return Err(err);
            }
        }
    }

    // Final attempt
    match operation() {
        Ok(value) => Ok(value),
        Err(err) => {
            error!(
                "Max retries exceeded while running {}, returning with the last error",
                name
            );
            Err(err)
        }
    }
}

pub trait Backoff {
    /// Base delay in ms.
    fn base_delay_ms(&self) -> u64;

    /// Generates next delay given current delay.
    fn next_delay_ms(&self, curr_delay_ms: u64) -> u64;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct HalfBackoff;

    impl Backoff for HalfBackoff {
        fn base_delay_ms(&self) -> u64 {
            128
        }

        fn next_delay_ms(&self, curr: u64) -> u64 {
            curr / 2
        }
    }

    #[test]
    fn tracks_sleep_and_retries_correctly() {
        let backoff = HalfBackoff;
        let counter = Arc::new(Mutex::new(0));
        let sleep_log = Arc::new(Mutex::new(Vec::new()));
        let max_retries = 2;

        let result = retry_with_backoff_inner(
            "mock_op",
            max_retries,
            &backoff,
            {
                let counter = Arc::clone(&counter);
                move || -> Result<(), &str> {
                    let mut count = counter.lock().unwrap();
                    *count += 1;
                    Err("fail")
                }
            },
            {
                let sleep_log = Arc::clone(&sleep_log);
                move |dur| {
                    sleep_log.lock().unwrap().push(dur.as_millis() as u64);
                }
            },
        );

        assert_eq!(result, Err("fail"));
        assert_eq!(*counter.lock().unwrap(), max_retries);
        assert_eq!(sleep_log.lock().unwrap().len(), (max_retries - 1) as usize);
        assert_eq!(sleep_log.lock().unwrap().to_vec(), vec![128]);
    }

    #[test]
    fn succeeds_after_retries() {
        let backoff = HalfBackoff;
        let attempts_counter = Arc::new(Mutex::new(0));
        let success_at_attempt = 1; // Succeeds on the 2nd attempt (0-indexed)
        let sleep_log = Arc::new(Mutex::new(Vec::new()));
        let max_retries = 3;

        let result = retry_with_backoff_inner(
            "mock_op_success",
            max_retries,
            &backoff,
            {
                let attempts_counter = Arc::clone(&attempts_counter);
                move || -> Result<&str, &str> {
                    let mut attempts = attempts_counter.lock().unwrap();
                    *attempts += 1;
                    if *attempts - 1 == success_at_attempt {
                        Ok("success")
                    } else {
                        Err("fail")
                    }
                }
            },
            {
                let sleep_log = Arc::clone(&sleep_log);
                move |dur| {
                    sleep_log.lock().unwrap().push(dur.as_millis() as u64);
                }
            },
        );

        assert_eq!(result, Ok("success"));
        assert_eq!(*attempts_counter.lock().unwrap(), success_at_attempt + 1);
        assert_eq!(sleep_log.lock().unwrap().len(), success_at_attempt);
        assert_eq!(sleep_log.lock().unwrap().to_vec(), vec![128]);
    }
}
