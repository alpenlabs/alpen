//! Strata client binary entrypoint.

use std::time::Duration;

use anyhow::{Result, anyhow};
use argh::from_env;
use strata_db_types as _;
use strata_logging::{LoggingInitConfig, init_logging_from_config};
#[cfg(test)]
use strata_ol_state_types as _;
#[cfg(test)]
use strata_predicate as _;
use tokio::runtime::{self, Handle};
use tracing::info;

use crate::{
    args::Args, context::init_node_context, errors::InitError, rpc::start_rpc,
    services::start_strata_services, startup_checks::run_startup_checks,
};

mod args;
mod config;
mod context;
mod errors;
mod genesis;
mod helpers;
mod init_db;
#[cfg(feature = "prover")]
mod prover;
mod rpc;
mod run_context;
#[cfg(feature = "sequencer")]
mod sequencer;
mod services;
mod startup_checks;

fn main() -> Result<()> {
    let args: Args = from_env();

    // Load config early to initialize logging with config settings
    let config = context::load_config_early(&args)
        .map_err(|e| anyhow!("Failed to load configuration: {e}"))?;

    // Init runtime. This needs to exist through the scope of main function so can't be created
    // inside `init_node_context`. Plus, logging also requires a handle to this.
    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("strata-rt")
        .build()
        .map_err(InitError::RuntimeBuild)?;

    // Initialize logging
    init_logging(rt.handle(), &config);

    // Validate sequencer flag isn't used when sequencer feature is disabled.
    #[cfg(not(feature = "sequencer"))]
    if args.sequencer {
        return Err(anyhow!(
            "Sequencer flag enabled but binary built without `sequencer` feature"
        ));
    }

    // Validate params, configs and create node context.
    let nodectx = init_node_context(&args, config.clone(), rt.handle().clone())
        .map_err(|e| anyhow!("Failed to initialize node context: {e}"))?;

    // Check for db consistency, external rpc clients reachable, etc.
    run_startup_checks(&nodectx)?;

    // Load sequencer key early so it can be shared with both the envelope writer
    // (for SPS-51 taproot authentication) and the duty executor (for block signing).
    #[cfg(feature = "sequencer")]
    let sequencer_key = if nodectx.config().client.is_sequencer {
        let path = args
            .sequencer_key
            .as_ref()
            .ok_or_else(|| anyhow!("--sequencer-key is required when --sequencer is set"))?;
        Some(sequencer::load_seqkey(path)?)
    } else {
        None
    };

    #[cfg(feature = "sequencer")]
    let sequencer_sk = sequencer_key.as_ref().map(|k| k.sk.0);

    #[cfg(not(feature = "sequencer"))]
    let sequencer_sk: Option<[u8; 32]> = None;

    // Start services, and do genesis if necessary.
    let (runctx, proof_notify) = start_strata_services(nodectx, sequencer_sk)?;

    // Start RPC.
    start_rpc(&runctx)?;

    // Start the integrated prover when the feature is enabled and a [prover]
    // section is present in the config. When absent, checkpoints use empty
    // proofs (requires AlwaysAccept predicate and Timeout publish mode).
    #[cfg(feature = "prover")]
    if let Some(proof_notify) = proof_notify {
        prover::start_prover_service(&runctx, runctx.executor(), proof_notify)?;
    }

    // Suppress unused variable warning when prover feature is disabled.
    #[cfg(not(feature = "prover"))]
    let _ = proof_notify;

    // Start sequencer signer if sequencer feature is enabled
    #[cfg(feature = "sequencer")]
    let _sequencer_monitor = if runctx.config().client.is_sequencer {
        Some(sequencer::start_sequencer_signer(&runctx, &args)?)
    } else {
        None
    };

    // Monitor tasks.
    runctx.task_manager.start_signal_listeners();
    runctx.task_manager.monitor(Some(Duration::from_secs(5)))?;

    info!("Exiting strata");
    Ok(())
}

fn init_logging(rt: &Handle, config: &strata_config::Config) {
    // Need to set the runtime context for async OTLP setup
    let _g = rt.enter();
    init_logging_from_config(LoggingInitConfig {
        service_base_name: "strata-client",
        service_label: config.logging.service_label.as_deref(),
        otlp_url: config.logging.otlp_url.as_deref(),
        log_dir: config.logging.log_dir.as_ref(),
        log_file_prefix: config.logging.log_file_prefix.as_deref(),
        json_format: config.logging.json_format,
        default_log_prefix: "alpen",
    });
}
