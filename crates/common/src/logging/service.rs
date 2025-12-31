//! Common logging service initialization for binaries.

use std::path::PathBuf;

use tracing::info;

use super::{format_service_name, init, FileLoggingConfig, LoggerConfig};

/// Configuration parameters for logging initialization.
#[derive(Debug)]
pub struct LoggingInitConfig<'a> {
    /// Base service name
    pub service_base_name: &'a str,
    /// Optional service label to append like prod or dev
    pub service_label: Option<&'a str>,
    /// OpenTelemetry OTLP endpoint URL
    pub otlp_url: Option<&'a str>,
    /// Directory for file-based logging
    pub log_dir: Option<&'a PathBuf>,
    /// Prefix for log file names
    pub log_file_prefix: Option<&'a str>,
    /// Use JSON format instead of compact
    pub json_format: Option<bool>,
    /// Default log file prefix if not specified in config
    pub default_log_prefix: &'a str,
}

/// Initialize logging from configuration with all standard setup.
///
/// This function encapsulates the common logging initialization logic used
/// across all binaries:
pub fn init_logging_from_config(config: LoggingInitConfig<'_>) {
    // Construct service name with optional label
    let service_name = format_service_name(config.service_base_name, config.service_label);

    let mut lconfig = LoggerConfig::new(service_name);

    // Configure OTLP if URL provided
    if let Some(url) = config.otlp_url {
        lconfig.set_otlp_url(url.to_string());
    }

    // Configure file logging if log directory provided
    let file_logging_config = config.log_dir.map(|dir| {
        let prefix = config
            .log_file_prefix
            .unwrap_or(config.default_log_prefix)
            .to_string();
        FileLoggingConfig::new(dir.clone(), prefix)
    });

    if let Some(file_config) = &file_logging_config {
        lconfig = lconfig.with_file_logging(file_config.clone());
    }

    // Configure JSON format if specified
    if let Some(json_format) = config.json_format {
        lconfig = lconfig.with_json_logging(json_format);
    }

    // Initialize logging
    init(lconfig);

    // Log configuration after init
    if let Some(url) = config.otlp_url {
        info!(%url, "using OpenTelemetry tracing output");
    }
    if let Some(file_config) = &file_logging_config {
        info!(
            log_dir = %file_config.directory.display(),
            log_prefix = %file_config.file_name_prefix,
            "file logging enabled"
        );
    }
}
