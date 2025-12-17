# Logging & Instrumentation Improvements for Vertex Core

**Date:** 2025-12-03
**Status:** Design Document

## Executive Summary

**Core Problem:** Actor-based async system produces jumbled logs where related events are scattered across services, making root cause analysis slow and painful.

**Solution:** Structured tracing with correlation IDs, semantic filtering, distributed trace propagation via jsonrpsee, and rate limiting for noisy loops.

**Key Benefits:**
- Follow a single request through all services with one grep command
- Filter logs by component/subsystem semantically
- Detect and trace slow operations automatically
- Full OpenTelemetry integration for Grafana visualization
- Backward compatible rollout strategy

---

## Current State Analysis

### What We Found

1. **Logging Infrastructure** (`logging.rs:42-83`):
   - Basic `tracing-subscriber` with env filter
   - Optional OpenTelemetry export to Grafana
   - No span context, no trace IDs, no request correlation
   - Just stdout + optional OTLP

2. **Usage Patterns**:
   - 408 logging calls across 91 files
   - **Only 1 use of `#[instrument]`** - not using structured tracing properly
   - 453 uses of anyhow error handling with `.context()` / `.with_context()`
   - Heavy use of abbreviated identifiers (e.g., `blkid=aa026e..91422d`)

3. **The Crash Log Problem**:
   - Shows `bail_manager` triggered shutdown with `ctx=duty_sign_block`
   - **Zero context about WHY it was triggered**
   - Can't trace back who sent the bail signal
   - Block processing looks normal until sudden shutdown
   - No timing data to identify slow operations

4. **Architecture**:
   - jsonrpsee-based RPC between services
   - Static service configuration
   - Centralized log aggregation (already in place)
   - OpenTelemetry export to Grafana (already configured)

### Example of Current Problem

```log
2025-12-03T07:13:37.782398Z  INFO handlemsg: strata_asm_worker::service: ASM found pivot anchor state pivot_block=100@30eb..7e34 service=asm_worker input=L1BlockCommitment(height=100, blkid=347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30)
2025-12-03T07:13:37.801195Z  INFO handlemsg: strata_asm_proto_bridge_v1::subprotocol: Successfully reassigned expired assignments count=0 deposits=[] service=asm_worker input=L1BlockCommitment(height=101, blkid=31da6bb9d50589812a7afcb6800240058057f53b72cf98b9579e035f1f041383)
2025-12-03T07:13:44.958568Z  WARN strata_common::bail_manager: tripped bail interrupt, exiting... ctx=duty_sign_block
```

**Problems:**
- Events from height=100 and height=101 are interleaved - can't follow a single L1 block's processing
- Can't correlate the bail interrupt to any prior event
- Abbreviated block IDs (`30eb..7e34`) are ungrepable
- No request ID to tie related events together

---

## Part 1: Enhanced OpenTelemetry Setup

Your current setup is basic. Let's upgrade it to capture distributed traces properly.

### Update `crates/common/src/logging.rs`

```rust
use opentelemetry::{trace::TracerProvider, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,  // ← ADD THIS
    Resource,
};
use tracing::*;
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},  // ← ADD FmtSpan
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
    Layer,
};

pub const OTLP_URL_ENVVAR: &str = "STRATA_OTLP_URL";
pub const SVC_LABEL_ENVVAR: &str = "STRATA_SVC_LABEL";

#[derive(Debug)]
pub struct LoggerConfig {
    whoami: String,
    otel_url: Option<String>,
    pub json_output: bool,  // ← ADD THIS for production
}

impl LoggerConfig {
    pub fn new(whoami: String) -> Self {
        Self {
            whoami,
            otel_url: None,
            json_output: false,
        }
    }

    pub fn with_base_name(s: &str) -> Self {
        Self::new(get_whoami_string(s))
    }

    pub fn set_otlp_url(&mut self, url: String) {
        self.otel_url = Some(url);
    }

    pub fn set_json_output(&mut self, enabled: bool) {
        self.json_output = enabled;
    }
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self::with_base_name("(strata-service)")
    }
}

/// Initializes the logging subsystem with the provided config.
pub fn init(config: LoggerConfig) {
    // Set global propagator for distributed tracing
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    let filt = EnvFilter::from_default_env()
        .add_directive("info".parse().unwrap());

    // Enhanced stdout logging with span events
    let stdout_sub = if config.json_output {
        // JSON format for production with full span context
        fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_target(true)
            .with_thread_ids(false)
            .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)  // Log span entry/exit with duration
            .with_filter(filt.clone())
            .boxed()
    } else {
        // Compact format for dev with span events
        fmt::layer()
            .compact()
            .with_target(true)
            .with_thread_ids(false)
            .with_span_events(FmtSpan::CLOSE)  // Log duration on close
            .with_filter(filt)
            .boxed()
    };

    // OpenTelemetry output with enhanced config
    if let Some(otel_url) = &config.otel_url {
        let trace_config =
            opentelemetry_sdk::trace::Config::default()
                .with_resource(Resource::new(vec![
                    KeyValue::new("service.name", config.whoami.clone()),
                    // ADD MORE RESOURCE ATTRIBUTES FOR GRAFANA:
                    KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                    KeyValue::new("deployment.environment",
                        std::env::var("DEPLOYMENT_ENV").unwrap_or_else(|_| "dev".into())),
                ]))
                .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn);  // Or adaptive

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

        let otel_sub = tracing_opentelemetry::layer()
            .with_tracer(tt)
            .with_tracked_inactivity(true);  // Track inactive spans

        tracing_subscriber::registry()
            .with(stdout_sub)
            .with(otel_sub)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(stdout_sub)
            .init();
    }

    info!(whoami = %config.whoami, "logging started");
}

/// Shuts down the logging subsystem, flushing files as needed and tearing down
/// resources.
pub fn finalize() {
    info!("shutting down logging");
    // Flush OpenTelemetry spans
    opentelemetry::global::shutdown_tracer_provider();
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
```

---

## Part 2: Trace Context Propagation

### Create `crates/common/src/tracing_context.rs`

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::Span;
use uuid::Uuid;

/// Trace context that flows across service boundaries via RPC
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceContext {
    /// Unique request ID for this logical operation
    pub request_id: String,

    /// W3C TraceContext headers for OpenTelemetry
    /// See: https://www.w3.org/TR/trace-context/
    #[serde(default)]
    pub traceparent: Option<String>,

    #[serde(default)]
    pub tracestate: Option<String>,

    /// Custom baggage for domain-specific context
    #[serde(default)]
    pub baggage: HashMap<String, String>,
}

impl TraceContext {
    /// Create a new trace context from current span
    pub fn from_current_span() -> Self {
        let request_id = Uuid::new_v4().to_string();

        // Extract OpenTelemetry context from current span
        let (traceparent, tracestate) = extract_otel_context();

        Self {
            request_id,
            traceparent,
            tracestate,
            baggage: HashMap::new(),
        }
    }

    /// Create a new root trace context
    pub fn new_root() -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            traceparent: None,
            tracestate: None,
            baggage: HashMap::new(),
        }
    }

    /// Attach domain-specific context
    pub fn with_baggage(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.baggage.insert(key.into(), value.into());
        self
    }

    /// Get short request ID for logging (first 8 chars)
    pub fn short_id(&self) -> &str {
        &self.request_id[..8.min(self.request_id.len())]
    }
}

/// Extract OpenTelemetry context from current span
fn extract_otel_context() -> (Option<String>, Option<String>) {
    use opentelemetry::global;
    use opentelemetry::propagation::TextMapPropagator;

    let propagator = global::get_text_map_propagator(|p| p.clone());
    let context = Span::current().context();

    let mut carrier = HashMap::new();
    propagator.inject_context(&context, &mut carrier);

    (
        carrier.get("traceparent").cloned(),
        carrier.get("tracestate").cloned(),
    )
}

/// Inject trace context into current span
pub fn inject_trace_context(ctx: &TraceContext) {
    use opentelemetry::global;
    use opentelemetry::propagation::TextMapPropagator;

    if let Some(ref traceparent) = ctx.traceparent {
        let mut carrier = HashMap::new();
        carrier.insert("traceparent".to_string(), traceparent.clone());
        if let Some(ref tracestate) = ctx.tracestate {
            carrier.insert("tracestate".to_string(), tracestate.clone());
        }

        let propagator = global::get_text_map_propagator(|p| p.clone());
        let parent_ctx = propagator.extract(&carrier);

        // Attach to current span
        Span::current().set_parent(parent_ctx);
    }
}
```

---

## Part 3: Instrumented RPC Client Wrapper

### Create `crates/common/src/instrumented_rpc.rs`

```rust
use async_trait::async_trait;
use jsonrpsee::core::{client::ClientT, ClientError};
use std::sync::Arc;
use tracing::*;
use crate::tracing_context::TraceContext;

/// Wrapper that automatically instruments RPC calls with tracing
pub struct InstrumentedRpcClient<C> {
    inner: Arc<C>,
    service_name: &'static str,
}

impl<C> InstrumentedRpcClient<C> {
    pub fn new(client: Arc<C>, service_name: &'static str) -> Self {
        Self {
            inner: client,
            service_name,
        }
    }
}

impl<C: ClientT> InstrumentedRpcClient<C> {
    /// Call RPC method with automatic tracing
    pub async fn call_with_trace<R, P>(
        &self,
        method: &str,
        params: P,
        trace_ctx: Option<TraceContext>,
    ) -> Result<R, ClientError>
    where
        R: serde::de::DeserializeOwned,
        P: serde::Serialize + Send,
    {
        // Get or create trace context
        let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::from_current_span);

        // Create span for this RPC call
        let span = info_span!(
            "rpc_call",
            component = "rpc_client",
            target_service = self.service_name,
            rpc_method = method,
            req_id = trace_ctx.short_id(),
            otel.kind = "client",  // OpenTelemetry semantic convention
        );

        let _enter = span.enter();

        debug!(
            rpc_method = method,
            target = self.service_name,
            "outbound RPC call"
        );

        // Wrap params with trace context
        let params_with_trace = TracedRpcParams {
            params,
            trace_ctx: Some(trace_ctx),
        };

        // Make the call
        let start = std::time::Instant::now();
        let result = self.inner.request(method, params_with_trace).await;
        let duration_ms = start.elapsed().as_millis();

        match &result {
            Ok(_) => {
                info!(
                    rpc_method = method,
                    duration_ms,
                    "RPC call succeeded"
                );
            }
            Err(e) => {
                error!(
                    rpc_method = method,
                    duration_ms,
                    error = %e,
                    "RPC call failed"
                );
            }
        }

        result
    }
}

/// Wrapper for RPC params that includes trace context
#[derive(serde::Serialize, serde::Deserialize)]
struct TracedRpcParams<P> {
    #[serde(flatten)]
    params: P,

    #[serde(skip_serializing_if = "Option::is_none")]
    trace_ctx: Option<TraceContext>,
}
```

---

## Part 4: RPC Server-Side Instrumentation

### Modify `crates/rpc/api/src/lib.rs`

Add trace context to method signatures (backward compatible):

```rust
use strata_common::tracing_context::TraceContext;

#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait StrataApi {
    /// Get blocks at a certain height
    #[method(name = "getBlocksAtIdx")]
    async fn get_blocks_at_idx(
        &self,
        idx: u64,
        trace_ctx: Option<TraceContext>,  // ← ADD TO ALL METHODS
    ) -> RpcResult<Vec<HexBytes32>>;

    // ... repeat for all methods
}
```

### Update Server Implementation in `bin/strata-client/src/rpc_server.rs`

```rust
use strata_common::tracing_context::{TraceContext, inject_trace_context};

#[async_trait]
impl StrataApiServer for StrataRpcImpl {
    async fn get_blocks_at_idx(
        &self,
        idx: u64,
        trace_ctx: Option<TraceContext>,
    ) -> RpcResult<Vec<HexBytes32>> {
        // Extract and inject trace context
        let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::new_root);
        inject_trace_context(&trace_ctx);

        // Create span with correlation
        let span = info_span!(
            "rpc_handler",
            component = "strata_rpc",
            rpc_method = "getBlocksAtIdx",
            req_id = trace_ctx.short_id(),
            otel.kind = "server",
            idx,
        );

        async move {
            info!(idx, "handling getBlocksAtIdx");

            // Your existing implementation
            self.storage
                .l2()
                .get_blocks_at_slot(idx)
                .await
                .map(|blocks| blocks.into_iter().map(HexBytes32).collect())
                .map_err(to_jsonrpsee_error)
        }
        .instrument(span)  // ← Attach span to async block
        .await
    }
}
```

---

## Part 5: Service Message Handler Instrumentation

### Pattern for All Services (Apply to `asm/worker`, `csm-worker`, `chain-worker`, etc.)

Example: `crates/asm/worker/src/service.rs`

```rust
use strata_common::tracing_context::TraceContext;

impl<W: WorkerContext + Send + Sync + 'static> SyncService for AsmWorkerService<W> {
    fn process_input(
        state: &mut AsmWorkerServiceState<W>,
        incoming_block: &L1BlockCommitment,
    ) -> anyhow::Result<Response> {
        // Create trace context for this processing
        let trace_ctx = TraceContext::new_root()
            .with_baggage("l1_height", incoming_block.height().to_string())
            .with_baggage("l1_block", incoming_block.blkid().to_string());

        // Create root span for this message
        let span = info_span!(
            "handlemsg",
            component = "asm_worker",
            service = "asm_worker",
            req_id = trace_ctx.short_id(),
            l1_height = incoming_block.height(),
            l1_block = %incoming_block.blkid(),  // FULL BLOCK ID, not abbreviated!
            trigger = "l1_block",
        );

        let _enter = span.enter();

        let ctx = &state.context;
        let genesis_height = state.params.rollup().genesis_l1_view.height();
        let height = incoming_block.height();

        if height < genesis_height {
            warn!(%height, "ignoring unexpected L1 block before genesis");
            return Ok(Response::Continue);
        }

        // Find pivot with sub-span
        let pivot_span = debug_span!(
            "find_pivot",
            component = "asm_worker",
            req_id = trace_ctx.short_id(),
        );
        let _pivot_enter = pivot_span.enter();

        let mut skipped_blocks = vec![];
        let mut pivot_block = *incoming_block;
        let mut pivot_anchor = ctx.get_anchor_state(&pivot_block);

        while pivot_anchor.is_err() && pivot_block.height() >= genesis_height {
            let block = ctx.get_l1_block(pivot_block.blkid())?;
            let parent_height = pivot_block.height().to_consensus_u32() - 1;
            let parent_block_id = L1BlockCommitment::from_height_u64(
                parent_height as u64,
                block.header.prev_blockhash.into(),
            )
            .expect("parent height should be valid");

            skipped_blocks.push((block, pivot_block));
            pivot_anchor = ctx.get_anchor_state(&parent_block_id);
            pivot_block = parent_block_id;
        }

        drop(_pivot_enter);  // Exit pivot span

        if pivot_block.height() < genesis_height {
            warn!("ASM hasn't found pivot anchor state at genesis.");
            return Ok(Response::ShouldExit);
        }

        info!(
            pivot_block = %pivot_block,
            skipped_count = skipped_blocks.len(),
            "ASM found pivot anchor state"
        );
        state.update_anchor_state(pivot_anchor.unwrap(), pivot_block);

        // Process blocks with individual spans
        for (block, block_id) in skipped_blocks.iter().rev() {
            let transition_span = info_span!(
                "asm_transition",
                component = "asm_worker",
                req_id = trace_ctx.short_id(),
                l1_height = block_id.height(),
                l1_block = %block_id,
            );
            let _trans_enter = transition_span.enter();

            info!("ASM transition attempt");

            match state.transition(block) {
                Ok(asm_stf_out) => {
                    let new_state = AsmState::from_output(asm_stf_out);
                    state.context.store_anchor_state(block_id, &new_state)?;
                    state.update_anchor_state(new_state, *block_id);
                    info!("ASM transition success");
                }
                Err(e) => {
                    error!(error = %e, "ASM transition error");
                    return Ok(Response::ShouldExit);
                }
            }
        }

        Ok(Response::Continue)
    }
}
```

---

## Part 6: Update Sequencer Client

### Modify `bin/strata-sequencer-client/src/duty_executor.rs`

```rust
use strata_common::{instrumented_rpc::InstrumentedRpcClient, tracing_context::TraceContext};

pub(crate) async fn duty_executor_worker<R>(
    rpc: Arc<R>,
    mut duty_rx: mpsc::Receiver<Duty>,
    handle: Handle,
    idata: IdentityData,
    epoch_gas_limit: Option<u64>,
) -> anyhow::Result<()>
where
    R: StrataSequencerApiClient + Send + Sync + 'static,
{
    // Wrap RPC client with instrumentation
    let instrumented_rpc = Arc::new(InstrumentedRpcClient::new(
        rpc,
        "strata-sequencer",
    ));

    let mut seen_duties = HashSet::new();
    let (failed_duties_tx, mut failed_duties_rx) = mpsc::channel::<DutyId>(8);

    loop {
        select! {
            duty = duty_rx.recv() => {
                if let Some(duty) = duty {
                    let duty_id = duty.generate_id();
                    if seen_duties.contains(&duty_id) {
                        debug!(%duty_id, "skipping already seen duty");
                        continue;
                    }
                    seen_duties.insert(duty.generate_id());

                    // Create trace context for this duty
                    let trace_ctx = TraceContext::from_current_span()
                        .with_baggage("duty_type", format!("{:?}", duty))
                        .with_baggage("duty_id", duty_id.to_string());

                    handle.spawn(handle_duty(
                        instrumented_rpc.clone(),
                        duty,
                        idata.clone(),
                        failed_duties_tx.clone(),
                        epoch_gas_limit,
                        trace_ctx,
                    ));
                } else {
                    return Ok(());
                }
            }
            failed_duty = failed_duties_rx.recv() => {
                if let Some(duty_id) = failed_duty {
                    warn!(%duty_id, "removing failed duty");
                    seen_duties.remove(&duty_id);
                }
            }
        }
    }
}

async fn handle_sign_block_duty<R>(
    rpc: Arc<InstrumentedRpcClient<R>>,
    duty: BlockSigningDuty,
    duty_id: DutyId,
    idata: &IdentityData,
    epoch_gas_limit: Option<u64>,
    trace_ctx: TraceContext,
) -> Result<(), DutyExecError>
where
    R: StrataSequencerApiClient + Send + Sync,
{
    let span = info_span!(
        "handle_sign_block_duty",
        component = "sequencer_client",
        req_id = trace_ctx.short_id(),
        %duty_id,
        parent_slot = duty.parent().slot(),
    );

    async move {
        let now = now_millis();
        if now < duty.target_ts() {
            warn!(%duty_id, %now, target = duty.target_ts(), "got duty too early; sleeping");
            tokio::time::sleep(tokio::time::Duration::from_millis(
                duty.target_ts() - now_millis(),
            ))
            .await;
        }

        // Call RPC with trace context
        let template = rpc
            .call_with_trace(
                "strata_getBlockTemplate",
                BlockGenerationConfig::new(duty.parent(), epoch_gas_limit),
                Some(trace_ctx.clone()),
            )
            .await
            .map_err(DutyExecError::GenerateTemplate)?;

        let id = template.template_id();
        info!(%duty_id, block_id = %id, "got block template");

        let signature = sign_header(template.header(), &idata.key);
        let completion = BlockCompletionData::from_signature(signature);

        rpc.call_with_trace(
                "strata_completeBlockTemplate",
                (template.template_id(), completion),
                Some(trace_ctx),
            )
            .await
            .map_err(DutyExecError::CompleteTemplate)?;

        info!(%duty_id, block_id = %id, "block signing complete");
        Ok(())
    }
    .instrument(span)
    .await
}
```

---

## Part 7: Fix the Bail Manager

### Update `crates/common/src/bail_manager.rs`

```rust
use std::sync::LazyLock;
use tokio::sync::watch;
use tracing::*;

pub const BAIL_DUTY_SIGN_BLOCK: &str = "duty_sign_block";

struct BailWatch {
    sender: watch::Sender<Option<String>>,
    receiver: watch::Receiver<Option<String>>,
}

/// Singleton manager for `watch::Sender` and `watch::Receiver` used to communicate bail-out
/// contexts.
static BAIL_MANAGER: LazyLock<BailWatch> = LazyLock::new(|| {
    let (sender, receiver) = watch::channel(None);
    BailWatch { sender, receiver }
});

/// Publicly accessible `watch::Sender` for broadcasting bail-out context updates.
pub static BAIL_SENDER: LazyLock<watch::Sender<Option<String>>> =
    LazyLock::new(|| BAIL_MANAGER.sender.clone());

/// Publicly accessible `watch::Receiver` for subscribing to bail-out context updates.
pub static BAIL_RECEIVER: LazyLock<watch::Receiver<Option<String>>> =
    LazyLock::new(|| BAIL_MANAGER.receiver.clone());

/// Context about why bail was triggered
#[derive(Debug, Clone)]
pub struct BailTriggerContext {
    pub reason: String,
    pub current_operation: String,
    pub state_snapshot: Option<String>,
}

/// Trigger a bail interrupt with context
#[track_caller]  // ← Captures caller location automatically
pub fn trigger_bail(ctx: &str, bail_ctx: BailTriggerContext) {
    let caller = std::panic::Location::caller();

    error!(
        bail_target = ctx,
        reason = %bail_ctx.reason,
        operation = %bail_ctx.current_operation,
        state = ?bail_ctx.state_snapshot,
        caller = %caller,
        thread = ?std::thread::current().id(),
        "🚨 TRIGGERING BAIL INTERRUPT"
    );

    BAIL_SENDER.send_replace(Some(ctx.to_string()));
}

/// Checks to see if we should bail out.
#[track_caller]
pub fn check_bail_trigger(ctx: &str) {
    if let Some(val) = BAIL_RECEIVER.borrow().clone() {
        let caller = std::panic::Location::caller();

        warn!(
            check_ctx = ctx,
            bail_target = %val,
            caller = %caller,
            thread = ?std::thread::current().id(),
            "⚠️  tripped bail interrupt check"
        );

        if ctx == val {
            error!(
                ctx,
                bail_target = %val,
                "🔥 BAIL INTERRUPT MATCH - EXITING"
            );

            // Give other tasks 100ms to log their state
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::process::exit(0);
        }
    }
}
```

---

## Part 8: Logging Conventions

### Mandatory Fields for Every Span

```rust
// Level 1: Service/Worker spans (top-level)
info_span!(
    "service_loop",
    component = "asm_worker",      // REQUIRED: Which subsystem
    service = "asm_worker",         // REQUIRED: Service name
    instance = ?service_id,         // Optional: For multiple instances
)

// Level 2: Operation spans (within a service)
info_span!(
    "process_l1_block",
    component = "asm_worker",
    req_id = %ctx.request_id.short(),  // REQUIRED: For correlation
    l1_height = height,                // REQUIRED: Primary entity ID
    l1_block = %blkid,                 // REQUIRED: Full ID (not abbreviated!)
)

// Level 3: Sub-operation spans
debug_span!(
    "validate_block",
    component = "asm_worker",
    req_id = %ctx.request_id.short(),
    validation_type = "pow",
)
```

### Field Naming Conventions

```rust
// ✅ GOOD: Consistent, filterable
l1_height = 100
l1_block = "347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30"  // FULL ID
l2_slot = 8
l2_block = "90683b4788d27bebc4630297f1efb2bd1e1b5c7bb214a30bb870d344ec126d52"
epoch = 1
req_id = "a1b2c3d4"
component = "fork_choice_manager"

// ❌ BAD: Inconsistent, ungrepable
blkid = "aa026e..91422d"  // Abbreviated - can't grep!
pivot_block = "100@30eb..7e34"  // Mixed format
block_id = "235@e9bf..7131"  // Inconsistent naming
```

### Error Context Standards

Every error MUST include:

1. **What failed** - operation name
2. **What we were processing** - entity IDs
3. **Why it failed** - error type and message
4. **What state we were in** - relevant state snapshot

```rust
// Use anyhow context religiously:
self.try_exec_block(block)
    .context("failed to execute block")
    .with_context(|| format!(
        "block_slot={}, block_id={}, parent={}",
        block.slot(),
        block.blkid(),
        parent_blkid
    ))?;

// Even better: structured errors
self.try_exec_block(block).map_err(|e| {
    error!(
        error = %e,
        l2_slot = block.slot(),
        l2_block = %block.blkid(),
        parent_block = %parent_blkid,
        component = "fork_choice_manager",
        "block execution failed"
    );
    e
})?;
```

---

## Part 9: Loop Detection & Rate Limiting

### Problem: Noisy Loops Fill Logs

Current pattern that can spam logs:

```rust
// BAD: Logs every iteration
while !done {
    debug!("have some duties cnt={}", duties.len());  // ← Spams if tight loop
    // ...
}
```

### Solution: Rate-Limited Logging

Create `crates/common/src/rate_limited_log.rs`:

```rust
use std::time::{Duration, Instant};
use std::sync::Mutex;
use std::collections::HashMap;

/// Rate limiter for high-frequency log points
pub struct RateLimiter {
    last_logged: Mutex<HashMap<&'static str, Instant>>,
    min_interval: Duration,
}

impl RateLimiter {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            last_logged: Mutex::new(HashMap::new()),
            min_interval,
        }
    }

    /// Returns true if this log point should emit
    pub fn should_log(&self, key: &'static str) -> bool {
        let mut map = self.last_logged.lock().unwrap();
        let now = Instant::now();

        if let Some(&last) = map.get(key) {
            if now.duration_since(last) < self.min_interval {
                return false;
            }
        }

        map.insert(key, now);
        true
    }
}

// Global rate limiters
lazy_static! {
    static ref DEBUG_LIMITER: RateLimiter = RateLimiter::new(Duration::from_secs(5));
    static ref TRACE_LIMITER: RateLimiter = RateLimiter::new(Duration::from_secs(1));
}

/// Rate-limited debug macro
#[macro_export]
macro_rules! debug_ratelimited {
    ($key:expr, $($arg:tt)*) => {
        if $crate::rate_limited_log::DEBUG_LIMITER.should_log($key) {
            tracing::debug!($($arg)*);
        }
    };
}

// Usage in hot loops:
loop {
    debug_ratelimited!(
        "duty_extractor_loop",  // ← Static key
        cnt = duties.len(),
        "have some duties"
    );

    // Or with counter:
    if DEBUG_LIMITER.should_log_with_count("duty_loop", 100) {
        debug!(total_iterations = iterations, "duty extractor loop status");
    }
}
```

---

## Part 10: Grafana Queries

### Tempo Queries (for traces)

```promql
# All traces for a specific service
{service.name="strata-client"}

# Traces with errors
{service.name="strata-client"} && status=error

# Traces for specific RPC method
{service.name="strata-client" && span.rpc_method="getBlockTemplate"}

# Traces for specific L1 block
{resource.l1_height="235"}

# Slow RPC calls (>1s)
{service.name="strata-client" && span.otel.kind="client"} | duration > 1s
```

### Loki Queries (for logs with trace correlation)

```logql
# All logs for a specific request
{service="strata-client"} | json | req_id="a1b2c3d4"

# Errors with trace context
{service="strata-client"} |= "ERROR" | json | req_id != ""

# RPC calls by method
{service="strata-client"} | json | rpc_method="completeBlockTemplate"

# Fork choice manager activity
{service="strata-client"} | json | component="fork_choice_manager"

# All logs for L1 block 235
{service="strata-client"} | json | l1_height="235"
```

### Recommended Grafana Dashboard Panels

1. **Request Flow Visualization** - Service Graph panel showing RPC call topology
2. **Error Rate by Component** - Counter of errors grouped by `component` field
3. **RPC Latency Distribution** - Histogram of `duration_ms` by `rpc_method`
4. **Active Traces** - Table showing in-flight requests with `req_id`, `component`, elapsed time

---

## Part 11: Implementation Checklist

### Phase 1: Core Infrastructure (Week 1)
- [ ] Update `logging.rs` with enhanced OpenTelemetry config
- [ ] Create `tracing_context.rs` module
- [ ] Create `instrumented_rpc.rs` wrapper
- [ ] Update `bail_manager.rs` with context
- [ ] Create `rate_limited_log.rs` module

### Phase 2: RPC Layer (Week 2)
- [ ] Add `trace_ctx` parameter to all RPC trait methods in `rpc/api`
- [ ] Update all RPC server implementations to inject context
- [ ] Wrap client usage with `InstrumentedRpcClient`
- [ ] Test trace propagation across one service boundary

### Phase 3: Service Workers (Week 3)
- [ ] Add spans to `asm_worker::process_input`
- [ ] Add spans to `csm_worker::process_input`
- [ ] Add spans to `chain_worker::process_input`
- [ ] Add spans to fork choice manager
- [ ] Add spans to consensus logic components

### Phase 4: High-Volume Paths (Week 4)
- [ ] Audit and rate-limit noisy loops in duty extractor
- [ ] Add sampling to high-frequency debug logs
- [ ] Use `#[instrument(skip_all)]` on hot paths
- [ ] Identify and fix abbreviated block ID usage

### Phase 5: Observability (Week 5)
- [ ] Set up Grafana dashboards with recommended panels
- [ ] Create runbook for trace-based debugging
- [ ] Add alerts for error rates and slow traces
- [ ] Document trace ID lookup procedure
- [ ] Performance testing and overhead measurement

---

## Part 12: Migration Strategy

### Backward Compatible Approach

1. **Make `trace_ctx` optional** in RPC signatures:
   ```rust
   async fn get_blocks_at_idx(
       &self,
       idx: u64,
       trace_ctx: Option<TraceContext>,  // ← Optional!
   ) -> RpcResult<Vec<HexBytes32>>;
   ```

2. **Generate root context on server** if not provided:
   ```rust
   let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::new_root);
   ```

3. **Gradually roll out** client instrumentation service by service

4. **Use environment variable** to enable/disable trace propagation:
   ```rust
   if std::env::var("STRATA_ENABLE_TRACE_PROPAGATION").is_ok() {
       // Use trace context
   }
   ```

### Testing Strategy

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[tokio::test]
    async fn test_trace_propagation() {
        // Set up tracing for test
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer());
        tracing::subscriber::set_global_default(subscriber).unwrap();

        // Create trace context
        let ctx = TraceContext::new_root();
        let req_id = ctx.request_id.clone();

        // Simulate RPC call
        let result = rpc_client.call_with_trace("test_method", (), Some(ctx)).await;

        // Verify trace ID appears in logs
        // (requires capturing log output)
    }
}
```

---

## Summary: Before & After

### Before (Current State)

```log
2025-12-03T07:13:37.782398Z INFO handlemsg: strata_asm_worker::service: ASM found pivot anchor state pivot_block=100@30eb..7e34 service=asm_worker input=L1BlockCommitment(height=100, blkid=347e16b7...)
2025-12-03T07:13:44.958568Z WARN strata_common::bail_manager: tripped bail interrupt, exiting... ctx=duty_sign_block
```

**Problems:**
- Can't correlate events
- Can't find root cause
- Abbreviated IDs are ungrepable
- No timing information
- No cross-service tracing

### After (With Instrumentation)

```json
{
  "timestamp": "2025-12-03T07:13:37.782398Z",
  "level": "INFO",
  "message": "ASM found pivot anchor state",
  "span": {"name": "handlemsg", "component": "asm_worker"},
  "fields": {
    "req_id": "a1b2c3d4",
    "l1_height": 100,
    "l1_block": "347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30",
    "pivot_block": "100@30eb7e34",
    "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
    "span_id": "00f067aa0ba902b7"
  }
}
```

**Now you can:**

```bash
# Find ALL events for this request across ALL services:
grep 'req_id=a1b2c3d4' */service.log

# Or in Grafana Loki:
{req_id="a1b2c3d4"}  # Shows complete timeline

# Visualize in Grafana Tempo:
# See flame graph of where time was spent
# See which service had the error
# See exact error chain with full context
```

### Example Crash Log (After Improvements)

```log
INFO component=sequencer_client req_id=f7e3a1c2 duty_id=duty_123: handling sign block duty
WARN component=sequencer_client req_id=f7e3a1c2 duty_id=duty_123: got duty too early; sleeping
INFO component=sequencer_client req_id=f7e3a1c2 rpc_method=getBlockTemplate: outbound RPC call
ERROR component=rpc_server req_id=f7e3a1c2 caller=duty_executor.rs:125: RPC call failed: connection timeout
ERROR req_id=f7e3a1c2 bail_target=duty_sign_block reason="RPC timeout" operation="signing block template" caller=duty_executor.rs:145: 🚨 TRIGGERING BAIL INTERRUPT
WARN check_ctx=duty_sign_block bail_target=duty_sign_block caller=sequencer.rs:89: ⚠️ tripped bail interrupt check
ERROR ctx=duty_sign_block bail_target=duty_sign_block: 🔥 BAIL INTERRUPT MATCH - EXITING
```

**Root cause obvious:** RPC timeout → bail trigger. All correlated by `req_id=f7e3a1c2`.

---

## Operational Usage Examples

### Finding Root Causes

```bash
# When you see an error, grab its req_id:
ERROR component=fork_choice_manager req_id=f7e3a1c2 "block execution failed"

# Then trace it backwards:
grep 'req_id=f7e3a1c2' service.log | sort

# This shows the ENTIRE causal chain across all services!
```

### Filtering by Component

```bash
# All fork choice manager activity:
grep 'component=fork_choice_manager' service.log

# All events for L1 block 235:
grep 'l1_height=235' service.log

# Follow a request through all services:
grep 'req_id=a1b2c3d4' service.log

# All RPC calls:
grep 'trigger=rpc' service.log

# All errors with context:
grep 'ERROR\|WARN' service.log | grep -A5 -B5 'req_id='
```

### Detecting Performance Issues

```bash
# Find slow operations (spans log duration on CLOSE):
grep 'CLOSE' service.log | grep 'duration=[0-9]\{4,\}'  # >1sec

# Identify noisy loops:
sort service.log | uniq -c | sort -n | tail -20
```

---

## Expected Benefits

1. **Debug Time Reduction**: From hours to minutes for most issues
2. **Root Cause Clarity**: Every error has full context trail
3. **Performance Visibility**: Automatic timing for all operations
4. **Service Topology**: Visual map of RPC call patterns in Grafana
5. **Proactive Monitoring**: Alerts on slow/failing operations
6. **Production Debugging**: Debug issues without reproducing locally
7. **Semantic Filtering**: Filter logs by component/operation/entity
8. **Cross-Service Tracing**: Follow requests through entire system

---

## Performance Overhead

Expected overhead with this instrumentation:

- **CPU**: 1-3% for span creation/destruction
- **Memory**: ~100 bytes per active span
- **Latency**: <1ms per span in hot path
- **Log Volume**: 2-3x increase (mitigated by rate limiting)

Mitigation strategies:
- Use `#[instrument(skip_all)]` on hot paths
- Sample high-frequency operations (1 in N)
- Rate-limit debug logs in loops
- Use async span recording (already enabled)
- Adaptive sampling in OpenTelemetry

---

## Questions & Next Steps

1. Review this design document with the team
2. Prioritize which services to instrument first
3. Set up test environment with Grafana dashboards
4. Create proof-of-concept with one service pair
5. Measure performance overhead in realistic workload
6. Roll out incrementally, service by service
7. Create operational runbooks for trace-based debugging
