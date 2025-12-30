//! Logging initialization and shutdown management.

use std::{sync::OnceLock, time::Duration};

use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator, trace::TracerProvider as SdkTracerProvider,
};
use tracing::*;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

use super::types::LoggerConfig;

/// Global tracer provider for proper shutdown
static TRACER_PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();

/// Initializes the logging subsystem with the provided config.
pub fn init(config: LoggerConfig) {
    // Set the global trace context propagator for distributed tracing
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    let filt = tracing_subscriber::EnvFilter::from_default_env();

    // Configure stdout logging with JSON or compact format
    let stdout_sub = if config.stdout_config.json_format {
        tracing_subscriber::fmt::layer()
            .json()
            .with_span_events(config.stdout_config.fmt_span)
            .with_filter(filt.clone())
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .compact()
            .with_span_events(config.stdout_config.fmt_span)
            .with_filter(filt.clone())
            .boxed()
    };

    // Build optional file logging layer
    let file_layer = config.file_logging_config.as_ref().map(|file_config| {
        let file_appender = tracing_appender::rolling::RollingFileAppender::new(
            file_config.rotation.clone(),
            &file_config.directory,
            &file_config.file_name_prefix,
        );

        if file_config.json_format {
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(file_appender)
                .with_ansi(false) // No color codes in files
                .with_filter(filt.clone())
                .boxed()
        } else {
            tracing_subscriber::fmt::layer()
                .compact()
                .with_writer(file_appender)
                .with_ansi(false) // No color codes in files
                .with_filter(filt.clone())
                .boxed()
        }
    });

    // Build optional OpenTelemetry layer
    let otel_layer = config.otel_url.as_ref().map(|otel_url| {
        let resource = config.resource.build_resource();

        // TODO (@voidash) modify here once we have more idea on span limits
        let trace_config = opentelemetry_sdk::trace::Config::default().with_resource(resource);

        // Configure exporter with timeout
        // tonic is a grpc exporter
        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(otel_url)
            .with_timeout(config.otlp_export_config.timeout);

        // The underlying tonic client has built-in retry logic.
        let tp = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(exporter)
            .with_trace_config(trace_config)
            .install_batch(opentelemetry_sdk::runtime::Tokio)
            .expect("init: failed to initialize opentelemetry pipeline");

        // Store tracer provider for shutdown
        if TRACER_PROVIDER.set(tp.clone()).is_err() {
            error!("Failed to set global tracer provider");
        }

        let tt = tp.tracer("alpen-tracer");
        tracing_opentelemetry::layer().with_tracer(tt)
    });

    // Register all layers - with() accepts Option<Layer> so this scales cleanly
    tracing_subscriber::registry()
        .with(stdout_sub)
        .with(file_layer)
        .with(otel_layer)
        .init();

    info!(
        service_name = %config.resource.service_name,
        service_version = ?config.resource.service_version,
        deployment_environment = ?config.resource.deployment_environment,
        "logging initialized"
    );
}

/// Shuts down the logging subsystem, flushing pending spans and tearing down resources.
///
/// This function should be called before application exit to ensure all spans are flushed
/// to the OTLP collector. It will timeout after 10 seconds.
pub fn finalize() {
    info!("shutting down logging");

    if let Some(provider) = TRACER_PROVIDER.get() {
        match provider.shutdown() {
            Ok(()) => {
                info!("tracer provider shut down successfully");
            }
            Err(e) => {
                error!("failed to shut down tracer provider: {:?}", e);
            }
        }
    } else {
        // No OTLP configured, nothing to shut down
        debug!("no tracer provider to shut down");
    }

    // Shutdown global text map propagator
    opentelemetry::global::shutdown_tracer_provider();
}
