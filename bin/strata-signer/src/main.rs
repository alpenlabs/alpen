//! Standalone sequencer signer for Strata.
//!
//! Connects to a sequencer node via RPC, fetches signing duties,
//! and submits signatures. Private keys never leave this process.

mod args;
mod config;
mod constants;
mod helpers;

use std::{
    fs,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use args::Args;
use config::SignerConfig;
use constants::SHUTDOWN_TIMEOUT_MS;
use helpers::load_seqkey;
use http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
use strata_common::{
    healthz::{start_health_check_server, HealthCheckState},
    ws_client::{ManagedWsClient, WsClientConfig},
};
use strata_logging::{
    format_service_name, init_logging_from_config_with_layers, LoggingInitConfig,
};
use strata_metrics::{MetricsConfig, MetricsInitConfig, MetricsLayer};
use strata_signer::SignerBuilder;
use strata_tasks::TaskManager;
use tokio::runtime::Builder;
use tracing::info;

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    // Load config from TOML file.
    let config_str = fs::read_to_string(&args.config)?;
    let config: SignerConfig = toml::from_str(&config_str)?;
    if config
        .sequencer_admin_bearer_token
        .expose_secret()
        .is_empty()
    {
        anyhow::bail!("sequencer_admin_bearer_token must be set and non-empty");
    }

    let runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name("signer-rt")
        .build()
        .context("failed to build signer tokio runtime")?;

    let handle = runtime.handle();

    // Init logging and metrics. Need runtime context for async exporter setup.
    let _g = handle.enter();
    let prometheus_listen_addr = config.logging.metrics_port.map(|port| {
        let host = config
            .logging
            .metrics_host
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        SocketAddr::from((host, port))
    });
    let metrics_config =
        MetricsConfig::from_exporters(config.logging.otlp_url.clone(), prometheus_listen_addr);
    let metrics_enabled = metrics_config.is_explicitly_enabled();
    let service_name =
        format_service_name("strata-signer", config.logging.service_label.as_deref());

    let mut extra_layers = Vec::new();
    if metrics_enabled {
        extra_layers.push(Box::new(MetricsLayer) as strata_logging::BoxedLayer);
    }

    init_logging_from_config_with_layers(
        LoggingInitConfig {
            service_base_name: "strata-signer",
            service_label: config.logging.service_label.as_deref(),
            otlp_url: config.logging.otlp_url.as_deref(),
            log_dir: config.logging.log_dir.as_ref(),
            log_file_prefix: config.logging.log_file_prefix.as_deref(),
            json_format: config.logging.json_format,
            default_log_prefix: "signer",
            extra_filter_directives: &["sp1_core_executor=warn", "jsonrpsee_server::server=warn"],
        },
        extra_layers,
    );

    let metrics_init = MetricsInitConfig::new(service_name).with_metrics_config(metrics_config);
    strata_metrics::init(metrics_init, handle).context("failed to initialize process metrics")?;

    let health_check_state = HealthCheckState::new();
    let health_check_addr = format!("{}:{}", config.health_check_host, config.health_check_port);
    let _health_check_handle = handle.block_on(start_health_check_server(
        health_check_addr.clone(),
        health_check_state.clone(),
    ))?;
    info!(%health_check_addr, "health check server started");

    // Load sequencer key. Raw bytes are zeroized inside load_seqkey before it returns.
    let (sk, pubkey) = load_seqkey(&config.sequencer_key)?;
    info!(?pubkey, "sequencer key loaded");

    // Set up RPC client.
    let ws_config = WsClientConfig {
        url: config.sequencer_admin_endpoint.clone(),
        headers: admin_auth_headers(config.sequencer_admin_bearer_token.expose_secret())?,
    };
    let rpc = Arc::new(ManagedWsClient::new_with_default_pool(ws_config));

    info!(sequencer_admin_endpoint = %config.sequencer_admin_endpoint, duty_poll_interval_ms = config.duty_poll_interval, "starting signer");

    // Launch signer service.
    let task_manager = TaskManager::new(handle.clone());
    let executor = task_manager.create_executor();

    let _monitor = handle.block_on(
        SignerBuilder::new(rpc, sk, Duration::from_millis(config.duty_poll_interval))
            .launch(&executor),
    )?;
    health_check_state.mark_ready();

    task_manager.start_signal_listeners();
    task_manager.monitor(Some(Duration::from_millis(SHUTDOWN_TIMEOUT_MS)))?;

    info!("exiting strata signer");
    strata_metrics::finalize();
    strata_logging::finalize();
    Ok(())
}

fn admin_auth_headers(token: &str) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&format!("Bearer {token}"))?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use http::header::HeaderName;

    use super::*;

    #[test]
    fn admin_auth_headers_sets_authorization() {
        let headers = admin_auth_headers("test-token").unwrap();
        assert_eq!(
            headers
                .get(HeaderName::from_static("authorization"))
                .unwrap(),
            "Bearer test-token"
        );
    }
}
