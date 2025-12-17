//! Unit tests for the logging subsystem.

use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry::global;
use opentelemetry::trace::TraceContextExt;
use std::collections::HashMap;


use super::types::*;

#[test]
fn test_resource_config_new() {
    let config = ResourceConfig::new("test-service".to_string());
    assert_eq!(config.service_name, "test-service");
    assert_eq!(config.service_version, None);
    assert_eq!(config.deployment_environment, None);
    assert_eq!(config.service_instance_id, None);
    assert!(config.custom_attributes.is_empty());
}

#[test]
fn test_resource_config_build_minimal() {
    let config = ResourceConfig::new("test-service".to_string());
    let resource = config.build_resource();

    // Resource should contain at least service.name
    let attrs: Vec<_> = resource.iter().collect();
    assert!(attrs
        .iter()
        .any(|(key, value)| key.as_str() == "service.name" && value.as_str() == "test-service"));
}

#[test]
fn test_resource_config_build_with_all_semantic_conventions() {
    let config = ResourceConfig {
        service_name: "test-service".to_string(),
        service_version: Some("1.0.0".to_string()),
        deployment_environment: Some("production".to_string()),
        service_instance_id: Some("instance-123".to_string()),
        custom_attributes: vec![
            KeyValue::new("custom.key1", "value1"),
            KeyValue::new("custom.key2", "value2"),
        ],
    };

    let resource = config.build_resource();
    let attrs: Vec<_> = resource.iter().collect();

    // Verify all semantic conventions are present
    assert!(attrs
        .iter()
        .any(|(key, value)| key.as_str() == "service.name" && value.as_str() == "test-service"));
    assert!(attrs
        .iter()
        .any(|(key, value)| key.as_str() == "service.version" && value.as_str() == "1.0.0"));
    assert!(attrs.iter().any(|(key, value)| key.as_str() == "deployment.environment"
        && value.as_str() == "production"));
    assert!(attrs.iter().any(|(key, value)| key.as_str() == "service.instance.id"
        && value.as_str() == "instance-123"));
    assert!(attrs
        .iter()
        .any(|(key, value)| key.as_str() == "custom.key1" && value.as_str() == "value1"));
    assert!(attrs
        .iter()
        .any(|(key, value)| key.as_str() == "custom.key2" && value.as_str() == "value2"));
}

#[test]
fn test_logger_config_builder_pattern() {
    let config = LoggerConfig::new("test-service".to_string())
        .with_service_version("2.0.0".to_string())
        .with_deployment_environment("staging".to_string())
        .with_service_instance_id("node-456".to_string())
        .with_json_logging(true)
        .add_resource_attribute("region", "us-west-2".to_string());

    assert_eq!(config.resource.service_name, "test-service");
    assert_eq!(config.resource.service_version, Some("2.0.0".to_string()));
    assert_eq!(
        config.resource.deployment_environment,
        Some("staging".to_string())
    );
    assert_eq!(
        config.resource.service_instance_id,
        Some("node-456".to_string())
    );
    assert!(config.stdout_config.json_format);
    assert_eq!(config.resource.custom_attributes.len(), 1);
}

#[test]
fn test_trace_context_propagation() {
    // Set up the propagator
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Create a mock trace context
    let mut carrier = HashMap::new();
    carrier.insert(
        "traceparent".to_string(),
        "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_string(),
    );

    // Extract context using the propagator
    let context = global::get_text_map_propagator(|propagator| propagator.extract(&carrier));

    // Verify context was extracted (we can't easily verify the exact trace ID
    // without more complex setup, but we can verify the extraction worked)
    assert!(context.span().span_context().is_valid());
}


#[test]
fn test_logger_config_with_otlp_export_config() {
    let export_config = OtlpExportConfig {
        timeout: Duration::from_secs(5),
        max_retries: 5,
    };

    let config =
        LoggerConfig::new("test-service".to_string()).with_otlp_export_config(export_config.clone());

    assert_eq!(config.otlp_export_config.timeout, export_config.timeout);
    assert_eq!(
        config.otlp_export_config.max_retries,
        export_config.max_retries
    );
}
