## Phase 2: Service Framework Auto-Instrumentation (CORRECTED)

**Objective:** Enhance existing service framework spans to provide consistent, production-visible observability for all services.

**Why Second:** This is the highest leverage change. The framework already instruments services, but spans are invisible in production (debug level) and use inconsistent field naming. Fix once, all services benefit.

**Dependency:** Phase 1 must be complete (FmtSpan::CLOSE enables duration logging for these spans).

**Effort:** 1-2 hours (single PR to strata-common)

---

### Background: What Already Exists

**Important Discovery:** The service framework in `strata-common` (commit 6948fec) **already instruments services**. We don't need to add instrumentation from scratch.

#### Current Infrastructure

**File:** `~/.cargo/git/checkouts/strata-common-e8492590f525284e/6948fec/crates/service/src/types.rs`

```rust
pub trait ServiceState: Sync + Send + 'static {
    /// Name for a service that can be printed in logs.
    fn name(&self) -> &str;
}
```

**All services already implement this:**
- `AsmWorkerServiceState::name()` returns `"asm_worker"` (crates/asm/worker/src/state.rs:130-132)
- `CsmWorkerState::name()` returns component name
- `ChainWorkerServiceState::name()` returns component name
- `ProverServiceState::name()` returns component name

**Framework already creates spans:**

`async_worker.rs:22-24`:
```rust
let service = state.name().to_owned();
let launch_span = debug_span!("onlaunch", %service);
S::on_launch(&mut state).instrument(launch_span).await?;
```

`async_worker.rs:57`:
```rust
let input_span = debug_span!("handlemsg", %service, ?input);
```

---

### The Real Problems

#### Problem 1: Spans Are Invisible in Production

**Current:** `debug_span!("onlaunch", ...)`
**Issue:** With `RUST_LOG=info` (production default), these spans never appear
**Fix:** Change `debug_span!` → `info_span!`

#### Problem 2: Inconsistent Field Naming

**Current:** Uses `%service` field
**Phase 1 Standard:** Uses `component` field
**Issue:** Filtering by component won't work (`grep 'component="asm_worker"'` returns nothing)
**Fix:** Rename `service` → `component`

#### Problem 3: Missing Semantic Conventions

**Current:** No `otel.kind` attribute
**Issue:** OpenTelemetry semantic conventions require span kind classification
**Fix:** Add `otel.kind = "internal"`

**Reference:** https://opentelemetry.io/docs/specs/semconv/trace/span-kind/

#### Problem 4: Poor Span Names

**Current:** `"onlaunch"`, `"handlemsg"`
**Issue:** Not descriptive, no namespace
**Fix:** `"service_launch"`, `"service_message"`

#### Problem 5: No Lifecycle Span

**Current:** Only spans individual operations (launch, message)
**Issue:** Can't see total service lifetime duration
**Fix:** Add `"service_worker"` span wrapping entire lifecycle

---

### Background Reading (Required)

#### 2.1 Existing Service Framework Code

**Read these files to understand current state:**

1. **Service traits**: `~/.cargo/git/checkouts/strata-common-e8492590f525284e/6948fec/crates/service/src/types.rs`
   - Lines 33-38: `ServiceState::name()` method
   - Lines 18-30: `Service` trait (State, Msg, Status)
   - Lines 63-86: `AsyncService` trait (process_input, on_launch, before_shutdown)

2. **Async worker**: `~/.cargo/git/checkouts/strata-common-e8492590f525284e/6948fec/crates/service/src/async_worker.rs`
   - Lines 10-43: `worker_task` function (main loop)
   - Lines 22-24: Launch span creation (debug level)
   - Lines 56-67: Message span creation (debug level)

3. **Sync worker**: `~/.cargo/git/checkouts/strata-common-e8492590f525284e/6948fec/crates/service/src/sync_worker.rs`
   - Lines 8-76: Blocking worker equivalent
   - Lines 22-24: Launch span (debug level)
   - Lines 37-48: Message span (debug level)

#### 2.2 Span Level Guidelines

**Read:**
- **Tracing Levels**: https://docs.rs/tracing/latest/tracing/struct.Level.html
  - ERROR: Serious errors
  - WARN: Potentially problematic situations
  - INFO: High-level informational messages (production default)
  - DEBUG: Lower-level debugging information (disabled in production)
  - TRACE: Very detailed trace information

**Key Insight:** Production systems run with `RUST_LOG=info`. Debug spans are invisible.

#### 2.3 OpenTelemetry Span Kind

**Read:**
- **Span Kind Spec**: https://opentelemetry.io/docs/specs/otel/trace/api/#spankind
  - `SERVER`: Request handler in an RPC system
  - `CLIENT`: Outbound request in an RPC system
  - `INTERNAL`: Internal operation (not RPC boundary)
  - `PRODUCER`: Message producer
  - `CONSUMER`: Message consumer

**Service workers are:** `INTERNAL` (not RPC boundaries, just internal message processing)

---

### Tasks

#### 2.1 Enhance Async Worker Spans

**File:** `~/.cargo/git/checkouts/strata-common-e8492590f525284e/6948fec/crates/service/src/async_worker.rs`

**Change 1: Add service lifecycle span** (wrap entire worker_task)

```rust
// BEFORE (lines 10-43)
pub(crate) async fn worker_task<S: AsyncService, I>(
    mut state: S::State,
    mut inp: I,
    status_tx: watch::Sender<S::Status>,
    shutdown_guard: strata_tasks::ShutdownGuard,
) -> anyhow::Result<()>
where
    I: AsyncServiceInput<Msg = S::Msg>,
{
    // Perform startup logic
    {
        let service = state.name().to_owned();
        let launch_span = debug_span!("onlaunch", %service);
        S::on_launch(&mut state).instrument(launch_span).await?;
    }

    // ... rest of function
}
```

```rust
// AFTER
pub(crate) async fn worker_task<S: AsyncService, I>(
    mut state: S::State,
    mut inp: I,
    status_tx: watch::Sender<S::Status>,
    shutdown_guard: strata_tasks::ShutdownGuard,
) -> anyhow::Result<()>
where
    I: AsyncServiceInput<Msg = S::Msg>,
{
    let component = state.name();  // Get component name once

    // NEW: Wrap entire service lifetime
    let service_span = info_span!(
        "service_worker",
        component = component,
        otel.kind = "internal",
    );

    async move {
        // Perform startup logic
        {
            let launch_span = info_span!(  // debug_span! → info_span!
                "service_launch",          // "onlaunch" → "service_launch"
                component = component,     // %service → component
                otel.kind = "internal",    // NEW: semantic convention
            );
            S::on_launch(&mut state).instrument(launch_span).await?;
        }

        // Wrapping for the worker task to respect shutdown requests
        let err = {
            let mut exit_fut = Box::pin(shutdown_guard.wait_for_shutdown().fuse());
            let mut wkr_fut =
                Box::pin(worker_task_inner::<S, I>(&mut state, &mut inp, &status_tx, component).fuse());

            futures::select! {
                _ = exit_fut => None,
                res = wkr_fut => res.err(),
            }
        };

        // Perform shutdown handling
        {
            let shutdown_span = info_span!(  // NEW: explicit shutdown span
                "service_shutdown",
                component = component,
                otel.kind = "internal",
            );
            handle_shutdown::<S>(&mut state, err.as_ref())
                .instrument(shutdown_span)
                .await;
        }

        Ok(())
    }
    .instrument(service_span)  // Wrap entire async block
    .await
}
```

**Change 2: Update message handling span** (lines 45-79)

```rust
// BEFORE
async fn worker_task_inner<S: AsyncService, I>(
    state: &mut S::State,
    inp: &mut I,
    status_tx: &watch::Sender<S::Status>,
) -> anyhow::Result<()>
where
    I: AsyncServiceInput<Msg = S::Msg>,
{
    let service = state.name().to_owned();

    while let Some(input) = inp.recv_next().await? {
        let input_span = debug_span!("handlemsg", %service, ?input);

        let res = match S::process_input(state, &input).instrument(input_span).await {
            Ok(res) => res,
            Err(e) => {
                error!(?input, %e, "failed to process message");
                return Err(e);
            }
        };

        // ... update status, check exit
    }

    Ok(())
}
```

```rust
// AFTER
async fn worker_task_inner<S: AsyncService, I>(
    state: &mut S::State,
    inp: &mut I,
    status_tx: &watch::Sender<S::Status>,
    component: &str,  // NEW: pass component name
) -> anyhow::Result<()>
where
    I: AsyncServiceInput<Msg = S::Msg>,
{
    while let Some(input) = inp.recv_next().await? {
        let input_span = info_span!(     // debug_span! → info_span!
            "service_message",           // "handlemsg" → "service_message"
            component = component,       // %service → component (consistent naming)
            otel.kind = "internal",      // NEW: semantic convention
            // Don't log full input - services add their own fields in process_input
        );

        let res = match S::process_input(state, &input).instrument(input_span).await {
            Ok(res) => res,
            Err(e) => {
                // Note: input_span is still active here, error will be associated
                error!(%e, "failed to process message");
                return Err(e);
            }
        };

        // Update status
        let status = S::get_status(state);
        let _ = status_tx.send(status);

        if res == Response::ShouldExit {
            break;
        }
    }

    Ok(())
}
```

**Change 3: Update shutdown handler** (lines 81-85)

```rust
// BEFORE
async fn handle_shutdown<S: AsyncService>(state: &mut S::State, err: Option<&anyhow::Error>) {
    if let Err(e) = S::before_shutdown(state, err).await {
        error!(%e, "unhandled error while shutting down");
    }
}
```

```rust
// AFTER (called from within shutdown_span in worker_task)
async fn handle_shutdown<S: AsyncService>(state: &mut S::State, err: Option<&anyhow::Error>) {
    // Shutdown span already active from caller
    if let Some(err) = err {
        warn!(%err, "service shutting down due to error");
    } else {
        info!("service shutting down gracefully");
    }

    if let Err(e) = S::before_shutdown(state, err).await {
        error!(%e, "unhandled error during shutdown cleanup");
    }
}
```

**Key Changes Summary:**
1. ✅ `debug_span!` → `info_span!` (visible in production)
2. ✅ `%service` → `component` (consistent field naming)
3. ✅ Add `otel.kind = "internal"` (semantic conventions)
4. ✅ Rename spans: `"onlaunch"` → `"service_launch"`, `"handlemsg"` → `"service_message"`
5. ✅ Add lifecycle span: `"service_worker"` wrapping entire service
6. ✅ Add explicit `"service_shutdown"` span

---

#### 2.2 Enhance Sync Worker Spans

**File:** `~/.cargo/git/checkouts/strata-common-e8492590f525284e/6948fec/crates/service/src/sync_worker.rs`

**Apply equivalent changes to sync worker:**

```rust
// BEFORE (lines 8-76)
pub(crate) fn worker_task<S: SyncService, I>(
    mut state: S::State,
    mut inp: I,
    status_tx: watch::Sender<S::Status>,
    shutdown_guard: strata_tasks::ShutdownGuard,
) -> anyhow::Result<()>
where
    I: SyncServiceInput<Msg = S::Msg>,
{
    let service = state.name().to_owned();

    // Perform startup logic
    {
        let launch_span = debug_span!("onlaunch", %service);
        let _g = launch_span.enter();
        S::on_launch(&mut state)?;
    }

    // Process messages...
    let mut err = None;
    while let Some(input) = inp.recv_next()? {
        if shutdown_guard.should_shutdown() {
            debug!("got shutdown notification");
            break;
        }

        let input_span = debug_span!("handlemsg", %service, ?input);
        let _g = input_span.enter();

        let res = match S::process_input(&mut state, &input) {
            Ok(res) => res,
            Err(e) => {
                error!(?input, %e, "failed to process message");
                err = Some(e);
                break;
            }
        };

        // ... status update, exit check
    }

    handle_shutdown::<S>(&mut state, err.as_ref());
    Ok(())
}
```

```rust
// AFTER
pub(crate) fn worker_task<S: SyncService, I>(
    mut state: S::State,
    mut inp: I,
    status_tx: watch::Sender<S::Status>,
    shutdown_guard: strata_tasks::ShutdownGuard,
) -> anyhow::Result<()>
where
    I: SyncServiceInput<Msg = S::Msg>,
{
    let component = state.name();

    // NEW: Wrap entire service lifetime
    let service_span = info_span!(
        "service_worker",
        component = component,
        otel.kind = "internal",
    );
    let _service_guard = service_span.enter();

    // Perform startup logic
    {
        let launch_span = info_span!(  // debug_span! → info_span!
            "service_launch",
            component = component,
            otel.kind = "internal",
        );
        let _g = launch_span.enter();
        S::on_launch(&mut state)?;
    }

    // Process messages
    let mut err = None;
    while let Some(input) = inp.recv_next()? {
        if shutdown_guard.should_shutdown() {
            info!("received shutdown signal");  // debug! → info!
            break;
        }

        let input_span = info_span!(  // debug_span! → info_span!
            "service_message",
            component = component,
            otel.kind = "internal",
        );
        let _g = input_span.enter();

        let res = match S::process_input(&mut state, &input) {
            Ok(res) => res,
            Err(e) => {
                error!(%e, "failed to process message");
                err = Some(e);
                break;
            }
        };

        if shutdown_guard.should_shutdown() {
            info!("received shutdown signal");
            break;
        }

        let status = S::get_status(&state);
        let _ = status_tx.send(status);

        if res == Response::ShouldExit {
            break;
        }
    }

    // Perform shutdown handling
    {
        let shutdown_span = info_span!(  // NEW: explicit shutdown span
            "service_shutdown",
            component = component,
            otel.kind = "internal",
        );
        let _g = shutdown_span.enter();
        handle_shutdown::<S>(&mut state, err.as_ref());
    }

    Ok(())
}

fn handle_shutdown<S: SyncService>(state: &mut S::State, err: Option<&anyhow::Error>) {
    if let Some(err) = err {
        warn!(%err, "service shutting down due to error");
    } else {
        info!("service shutting down gracefully");
    }

    if let Err(e) = S::before_shutdown(state, err) {
        error!(%e, "unhandled error during shutdown cleanup");
    }
}
```

---

#### 2.3 Create PR to strata-common

**PR Title:** Enhance service framework spans for production observability

**PR Description:**

```markdown
## Summary

Enhances existing service worker spans to be visible and useful in production:

1. **Visibility**: `debug_span!` → `info_span!` (visible with `RUST_LOG=info`)
2. **Consistency**: Rename `service` field → `component` (matches observability standards)
3. **Semantic conventions**: Add `otel.kind = "internal"` to all spans
4. **Clarity**: Better span names (`service_launch`, `service_message`, `service_shutdown`)
5. **Lifecycle tracking**: Add `service_worker` span wrapping entire service lifetime

## Motivation

Current spans are invisible in production (`debug` level) and use inconsistent field naming.
All services get automatic observability improvements from this change.

## Changes

### Files Modified

- `crates/service/src/async_worker.rs`
  - Add `service_worker` lifecycle span
  - Upgrade `debug_span!` → `info_span!`
  - Rename field: `%service` → `component`
  - Add `otel.kind = "internal"`
  - Rename spans for clarity

- `crates/service/src/sync_worker.rs`
  - Equivalent changes for blocking workers

### Breaking Changes

None. Changes are internal to span attributes. Services already implement `ServiceState::name()`.

## Testing

Verified span output:

```bash
RUST_LOG=info cargo test

# Output now shows:
# INFO service_worker{component="test_service"}: launching
# INFO service_launch{component="test_service"}: close time.busy=5ms
# INFO service_message{component="test_service"}: processing message
# INFO service_message{component="test_service"}: close time.busy=123ms
# INFO service_shutdown{component="test_service"}: close time.busy=10ms
# INFO service_worker{component="test_service"}: close time.busy=138ms
```

(Note: Duration logging requires consuming project to implement Phase 1 - FmtSpan::CLOSE)

## References

- OpenTelemetry Span Kind: https://opentelemetry.io/docs/specs/otel/trace/api/#spankind
- Tracing Levels: https://docs.rs/tracing/latest/tracing/struct.Level.html
```

**Checklist before submitting PR:**
- [ ] Test with both AsyncService and SyncService implementations
- [ ] Verify spans appear with `RUST_LOG=info`
- [ ] Check field naming: `component` not `service`
- [ ] Confirm `otel.kind = "internal"` on all spans
- [ ] Run existing tests to ensure no breakage

---

#### 2.4 Update vertex-core Dependency

**After strata-common PR merged:**

**File:** `Cargo.toml`

```toml
[workspace.dependencies]
# Update to new commit hash
strata-service = { git = "https://github.com/alpenlabs/strata-common", rev = "<new-commit-hash>" }
```

**Run:**
```bash
cargo update -p strata-service
cargo build
```

---

#### 2.5 Verification

**Test 1: Spans are visible in production log level**

```bash
RUST_LOG=info cargo run --bin strata-client 2>&1 | grep "service_"

# Expected output:
# INFO service_worker{component="asm_worker"}: launching service
# INFO service_launch{component="asm_worker"}: close time.busy=5ms
# INFO service_message{component="asm_worker"}: processing message
# INFO service_message{component="asm_worker"}: close time.busy=123ms
# INFO service_worker{component="asm_worker"}: close time.busy=138ms
```

**Test 2: Field naming is consistent**

```bash
RUST_LOG=info cargo run --bin strata-client 2>&1 | grep 'component="asm_worker"' | wc -l

# Should see multiple lines (every span from ASM worker)
```

**Test 3: Lifecycle span captures full duration**

```bash
# Look for service_worker span - should have longest duration
RUST_LOG=info cargo run --bin strata-client 2>&1 | grep "service_worker.*close"

# INFO service_worker{component="asm_worker"}: close time.busy=1234ms
```

**Test 4: Filter by component works**

```bash
# View all activity from a single component
RUST_LOG=info cargo run --bin strata-client 2>&1 | grep 'component="asm_worker"'

# Should see:
# - service_worker (lifecycle)
# - service_launch (startup)
# - service_message (each message)
# - service_shutdown (cleanup)
```

**Test 5: OpenTelemetry export includes otel.kind**

If OTLP is configured:
```bash
# Check Grafana/Jaeger for span attributes
# Each span should have: otel.kind = "internal"
```

---

### What We're NOT Doing (Removed from Original Phase 2)

❌ **NOT adding `component_name()` static method** - Already have `ServiceState::name()` instance method
❌ **NOT requiring services to implement new trait methods** - Already implemented
❌ **NOT creating GitHub issue** - Direct PR with clear changes
❌ **NOT 2 days of coordination** - 1-2 hours for straightforward enhancement

---

### Summary: Phase 2 Corrected Approach

**Old Plan (Incorrect):**
- Add new `Service::component_name()` method
- Each service implements it
- Framework uses it for spans
- Open issue, coordinate, 2 days effort

**New Plan (Correct):**
- Use existing `ServiceState::name()` method
- Enhance existing spans in framework
- Fix span levels, field names, add semantic conventions
- Direct PR, 1-2 hours effort

**Why Corrected Approach is Better:**
1. ✅ No breaking changes (no new trait methods)
2. ✅ Leverages existing infrastructure
3. ✅ Faster (1-2 hours vs 2 days)
4. ✅ Simpler (just enhance, not add)
5. ✅ All services benefit immediately

---

### Effort Estimate

**Time:** 1-2 hours (1 engineer)
- Code changes: 30 min (straightforward span enhancements)
- Testing: 30 min (verify span output)
- PR review: 30 min
- Dependency update: 15 min

**Complexity:** Low (well-understood changes to existing code)

**Risk:** Very low (no breaking changes, only internal span attributes)

---

### References

**strata-common source:**
- Repository: https://github.com/alpenlabs/strata-common
- Current commit: 6948fecf9c56669c0ec776ff9bb485f904ae163f
- Files to modify:
  - `crates/service/src/async_worker.rs`
  - `crates/service/src/sync_worker.rs`

**Documentation:**
- Tracing Levels: https://docs.rs/tracing/latest/tracing/struct.Level.html
- OpenTelemetry Span Kind: https://opentelemetry.io/docs/specs/otel/trace/api/#spankind
- Instrument: https://docs.rs/tracing/latest/tracing/trait.Instrument.html
