//! Logging subsystem with OpenTelemetry support.

pub mod manager;
pub mod types;

#[cfg(test)]
mod tests;

// Re-export main types and functions
pub use manager::{finalize, finalize_with_timeout, init};
pub use types::{FileLoggingConfig, LoggerConfig, OtlpExportConfig, ResourceConfig, StdoutConfig};

// Re-export tracing-appender types for convenience
pub use tracing_appender::rolling::Rotation;

/// Formats a service name with an optional label suffix.
pub fn format_service_name(base: &str, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("{base}%{label}"),
        None => base.to_owned(),
    }
}
