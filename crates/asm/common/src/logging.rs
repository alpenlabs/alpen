//! zkVM-compatible logging macros for ASM crates.
//!
//! This module provides logging macros that work in both regular and zkVM environments:
//! - In regular builds: Uses the real `tracing` crate for full logging functionality
//! - In zkVM builds: Provides zero-cost no-op macros that validate format strings at compile time

// When NOT building for zkVM, use real tracing macros
#[cfg(not(target_os = "zkvm"))]
pub use tracing::{debug, error, info, trace, warn};

// When building for zkVM, define zero-cost no-op macros
#[cfg(target_os = "zkvm")]
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}
}

#[cfg(target_os = "zkvm")]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}
}

#[cfg(target_os = "zkvm")]
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}
}

#[cfg(target_os = "zkvm")]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}
}

#[cfg(target_os = "zkvm")]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}
}

// Re-export the macros for zkVM builds so they can be used with the same syntax
#[cfg(target_os = "zkvm")]
pub use crate::{debug, error, info, trace, warn};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logging_macros_compile() {
        // This test ensures that our logging macros compile correctly and accept various argument
        // patterns
        let transaction_id = "tx123";
        let error_code = 42;

        // Test basic string logging
        info!("Processing transaction");

        // Test formatted logging with different argument types
        warn!(
            "Transaction {} failed with error code {}",
            transaction_id, error_code
        );

        // Test error logging with Debug formatting
        error!("Critical error: {:?}", ("complex", "data", 123));

        // Test debug and trace (should compile but may be no-op depending on target)
        debug!("Debug info: {}", "details");
        trace!("Trace data: {:x}", 255);

        // Verify that the macros handle expressions correctly
        let result = {
            info!("Starting operation");
            "success"
        };
        assert_eq!(result, "success");
    }

    #[test]
    fn test_format_args_validation() {
        // This test ensures that format_args! properly validates our format strings
        // even in zkVM mode where logging is a no-op

        // These should compile fine
        warn!("Simple message");
        error!("Message with arg: {}", 42);
        info!("Multiple args: {} and {}", "first", "second");

        // Test different format specifiers
        debug!(
            "Number: {}, hex: {:x}, debug: {:?}",
            123,
            255,
            vec![1, 2, 3]
        );
    }
}
