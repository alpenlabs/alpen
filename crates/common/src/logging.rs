use std::env;

use opentelemetry::{trace::TracerProvider, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use tracing::*;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

pub const OTLP_URL_ENVVAR: &str = "STRATA_OTLP_URL";
pub const SVC_LABEL_ENVVAR: &str = "STRATA_SVC_LABEL";

#[derive(Debug)]
pub struct LoggerConfig {
    whoami: String,
    otel_url: Option<String>,
}

impl LoggerConfig {
    /// Creates a new empty instance with whoami set.
    pub fn new(whoami: String) -> Self {
        Self {
            whoami,
            otel_url: None,
        }
    }

    pub fn with_base_name(s: &str) -> Self {
        Self::new(get_whoami_string(s))
    }

    pub fn set_otlp_url(&mut self, url: String) {
        self.otel_url = Some(url);
    }
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self::with_base_name("(strata-service)")
    }
}

/// Initializes the logging subsystem with the provided config.
pub fn init(config: LoggerConfig) {
    let filt = tracing_subscriber::EnvFilter::from_default_env();

    // TODO switch to using subscribers everywhere instead of layers
    //let mut loggers: Vec<Box<dyn tracing::Subscriber + 'static>> = Vec::new();

    // Stdout logging.
    let stdout_sub = tracing_subscriber::fmt::layer().compact().with_filter(filt);

    // OpenTelemetry output.
    if let Some(otel_url) = &config.otel_url {
        let trace_config =
            opentelemetry_sdk::trace::Config::default().with_resource(Resource::new(vec![
                KeyValue::new("service.name", config.whoami.clone()),
            ]));

        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(otel_url);

        let tp = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(exporter)
            .with_trace_config(trace_config)
            .install_batch(opentelemetry_sdk::runtime::Tokio)
            .expect("init: opentelemetry");

        let tt = tp.tracer("strata-log");

        let otel_sub = tracing_opentelemetry::layer().with_tracer(tt);

        tracing_subscriber::registry()
            .with(stdout_sub)
            .with(otel_sub)
            .init();
    } else {
        tracing_subscriber::registry().with(stdout_sub).init();
    }

    info!(whoami = %config.whoami, "logging started");
}

/// Shuts down the logging subsystem, flushing files as needed and tearing down
/// resources.
pub fn finalize() {
    info!("shutting down logging");
    // TODO
}

/// Gets the OTLP URL from the standard envvar.
pub fn get_otlp_url_from_env() -> Option<String> {
    env::var(OTLP_URL_ENVVAR).ok()
}

/// Gets the service label from the standard envvar, which should be included
/// in the whoami string.
pub fn get_service_label_from_env() -> Option<String> {
    env::var(SVC_LABEL_ENVVAR).ok()
}

/// Computes a standard whoami string.
pub fn get_whoami_string(base: &str) -> String {
    match get_service_label_from_env() {
        Some(label) => format!("{base}%{label}"),
        // Clippy is mad at me about this being `format!`.
        None => base.to_owned(),
    }
}
