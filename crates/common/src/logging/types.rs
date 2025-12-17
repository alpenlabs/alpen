//! Configuration types for the logging subsystem.

use std::{path::PathBuf, time::Duration};

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use tracing_appender::rolling::Rotation;
use tracing_subscriber::fmt::format::FmtSpan;

/// Configuration for the stdout/stderr logging layer
#[derive(Debug, Clone)]
pub struct StdoutConfig {
    /// Use JSON format instead of compact format
    pub json_format: bool,
    /// Span events to log (ENTER, EXIT, CLOSE, etc.)
    pub fmt_span: FmtSpan,
}

impl Default for StdoutConfig {
    fn default() -> Self {
        Self {
            json_format: false,
            // Log CLOSE events to capture span duration
            fmt_span: FmtSpan::CLOSE,
        }
    }
}

/// Configuration for file-based logging with rotation
#[derive(Debug, Clone)]
pub struct FileLoggingConfig {
    /// Directory where log files will be written
    pub directory: PathBuf,
    /// Base filename prefix (e.g., "alpen" -> "alpen.log")
    pub file_name_prefix: String,
    /// Rotation strategy (daily, hourly, never, size-based)
    pub rotation: Rotation,
    /// Use JSON format for file logs (default: false, uses compact)
    pub json_format: bool,
}

impl FileLoggingConfig {
    pub fn new(directory: PathBuf, file_name_prefix: String) -> Self {
        Self {
            directory,
            file_name_prefix,
            rotation: Rotation::DAILY,
            json_format: false,
        }
    }

    pub fn with_rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_json_format(mut self, json_format: bool) -> Self {
        self.json_format = json_format;
        self
    }
}

/// Configuration for OTLP exporter retry and timeout
#[derive(Debug, Clone)]
pub struct OtlpExportConfig {
    /// Timeout for export requests
    pub timeout: Duration,
    /// Maximum number of retry attempts
    pub max_retries: u32,
}

impl Default for OtlpExportConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_retries: 3,
        }
    }
}

/// Resource attributes following OpenTelemetry semantic conventions
#[derive(Debug, Clone)]
pub struct ResourceConfig {
    /// Service name (required)
    pub service_name: String,
    /// Service version (recommended)
    pub service_version: Option<String>,
    /// Deployment environment (e.g., "production", "staging", "development")
    pub deployment_environment: Option<String>,
    /// Service instance ID (unique identifier for this instance)
    pub service_instance_id: Option<String>,
    /// Additional custom attributes
    pub custom_attributes: Vec<KeyValue>,
}

impl ResourceConfig {
    pub fn new(service_name: String) -> Self {
        Self {
            service_name,
            service_version: None,
            deployment_environment: None,
            service_instance_id: None,
            custom_attributes: Vec::new(),
        }
    }

    /// Build OpenTelemetry Resource from config
    pub fn build_resource(&self) -> Resource {
        let ResourceConfig {
            service_name,
            service_version,
            deployment_environment,
            service_instance_id,
            custom_attributes,
        } = self;

        let mut attributes = vec![KeyValue::new("service.name", service_name.clone())];

        if let Some(version) = service_version {
            attributes.push(KeyValue::new("service.version", version.clone()));
        }

        if let Some(env) = deployment_environment {
            attributes.push(KeyValue::new("deployment.environment", env.clone()));
        }

        if let Some(instance_id) = service_instance_id {
            attributes.push(KeyValue::new("service.instance.id", instance_id.clone()));
        }

        attributes.extend(custom_attributes.iter().cloned());

        Resource::new(attributes)
    }
}

/// Main logger configuration
#[derive(Debug, Clone)]
pub struct LoggerConfig {
    /// Resource configuration
    pub resource: ResourceConfig,
    /// OTLP endpoint URL
    pub otel_url: Option<String>,
    /// Stdout logging configuration
    pub stdout_config: StdoutConfig,
    /// File logging configuration (optional)
    pub file_logging_config: Option<FileLoggingConfig>,
    /// OTLP export configuration
    pub otlp_export_config: OtlpExportConfig,
}

impl LoggerConfig {
    /// Creates a new configuration with service name
    pub fn new(service_name: String) -> Self {
        Self {
            resource: ResourceConfig::new(service_name),
            otel_url: None,
            stdout_config: StdoutConfig::default(),
            file_logging_config: None,
            otlp_export_config: OtlpExportConfig::default(),
        }
    }

    /// Set OTLP endpoint URL
    pub fn set_otlp_url(&mut self, url: String) {
        self.otel_url = Some(url);
    }

    /// Set service version
    pub fn with_service_version(mut self, version: String) -> Self {
        self.resource.service_version = Some(version);
        self
    }

    /// Set deployment environment
    pub fn with_deployment_environment(mut self, env: String) -> Self {
        self.resource.deployment_environment = Some(env);
        self
    }

    /// Set service instance ID
    pub fn with_service_instance_id(mut self, instance_id: String) -> Self {
        self.resource.service_instance_id = Some(instance_id);
        self
    }

    /// Enable JSON logging format
    pub fn with_json_logging(mut self, enabled: bool) -> Self {
        self.stdout_config.json_format = enabled;
        self
    }

    /// Enable file logging with configuration
    pub fn with_file_logging(mut self, config: FileLoggingConfig) -> Self {
        self.file_logging_config = Some(config);
        self
    }

    /// Configure which span events to log
    pub fn with_fmt_span(mut self, fmt_span: FmtSpan) -> Self {
        self.stdout_config.fmt_span = fmt_span;
        self
    }

    /// Set OTLP export configuration
    pub fn with_otlp_export_config(mut self, config: OtlpExportConfig) -> Self {
        self.otlp_export_config = config;
        self
    }

    /// Add custom resource attribute
    pub fn add_resource_attribute(mut self, key: &str, value: String) -> Self {
        self.resource
            .custom_attributes
            .push(KeyValue::new(key.to_string(), value));
        self
    }
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self::new("(strata-service)".to_string())
    }
}
