# PaaS Troubleshooting Guide

**Last Updated:** 2025-11-04

## Table of Contents

1. [Common Issues](#common-issues)
2. [State Machine Errors](#state-machine-errors)
3. [Test Failures](#test-failures)
4. [Performance Issues](#performance-issues)
5. [Debugging Tools](#debugging-tools)
6. [Log Analysis](#log-analysis)

---

## Common Issues

### Issue 1: Invalid State Transition Errors

**Symptoms:**
```
ERROR Failed to mark task as completed
  e=Unexpected("invalid status transition from Pending to Completed")
```

**Root Cause:**
Worker pool attempting to skip required state transitions.

**Solution:**
Ensure proper state progression:
```rust
// ✅ CORRECT
prover_handle.mark_queued(task_id).await?;
prover_handle.mark_proving(task_id).await?;
operator.process_proof(proof_key, &db).await?;
prover_handle.mark_completed(task_id).await?;

// ❌ INCORRECT
prover_handle.mark_completed(task_id).await?; // Skips Queued → Proving
```

**Valid Transitions:**
- `Pending → Queued → Proving → Completed`
- `Pending → Queued → Proving → TransientFailure → Queued` (retry)

**Fixed In:** Commit `baea23cf1` (2025-11-04)

---

### Issue 2: RPC Error -32001 "Failed to Create Task"

**Symptoms:**
```
RpcError: code -32001
Message: "failed to create task for cl block"
```

**Root Cause:**
Underlying state machine issue preventing task creation.

**Solution:**
1. Check service logs for state transition errors
2. Verify ProverService is running
3. Check database connectivity
4. Ensure dependencies are resolved

**Debugging:**
```bash
# Check service status
grep "PaaS service launched" service.log

# Check for errors
grep -i error service.log | tail -20

# Check task creation attempts
grep "Created proof task" service.log
```

---

### Issue 3: Proof Generation Timeout

**Symptoms:**
```
TimeoutError: Proof generation timed out after 30 seconds
```

**Common Causes:**
1. State machine blocking task progression
2. Worker pool not picking up tasks
3. Proof operator hanging
4. Resource exhaustion

**Solution:**
```bash
# Check if tasks are moving through states
grep "task_id=<id>" service.log

# Expected progression:
# - "marked as queued"
# - "marked as proving"
# - "Proof generation completed"
# - "marked as completed"

# Check worker pool activity
grep "Worker pool started" service.log
grep "Starting proof generation" service.log
```

---

### Issue 4: Worker Pool Not Processing Tasks

**Symptoms:**
- Tasks stuck in Pending state
- No "Starting proof generation" log entries
- Worker pool logs show no activity

**Common Causes:**
1. Worker count misconfigured (all zeros)
2. Worker pool not started
3. Polling interval too long
4. Tasks have unresolved dependencies

**Solution:**
```bash
# Check worker pool configuration
grep "worker_count" config.toml

# Should show non-zero values:
# worker_count = { Native = 4, SP1 = 2 }

# Check worker pool startup
grep "Worker pool started" service.log

# Check pending tasks
grep "pending_tasks" service.log
```

**Configuration Fix:**
```toml
[workers]
worker_count = { Native = 4, SP1 = 2 }
polling_interval_ms = 100
```

---

### Issue 5: Task Stuck in Queued State

**Symptoms:**
- Task transitions to Queued
- Never transitions to Proving
- No proof generation activity

**Common Causes:**
1. Worker limit reached for backend
2. Worker pool crashed
3. ProofOperator not properly initialized

**Solution:**
```bash
# Check worker limit
grep "Worker limit reached" service.log

# Check for worker crashes
grep -E "(panic|SIGTERM)" service.log

# Check proof operator initialization
grep "ProofOperator" service.log
```

---

### Issue 6: Excessive Retries

**Symptoms:**
- Tasks repeatedly marked as TransientFailure
- Same task retried multiple times
- Eventually marked as Failed after max retries

**Common Causes:**
1. Persistent proof generation error
2. Database issues
3. Resource constraints
4. Network issues (if remote backend)

**Solution:**
```bash
# Check retry count
grep "marked as transient failure" service.log

# Check error messages
grep "Proof generation failed" service.log

# Check max retries configuration
grep "max_retries" config.toml
```

**Configuration:**
```toml
[retry]
max_retries = 3
base_delay_ms = 100
max_delay_ms = 5000
```

---

## State Machine Errors

### Valid State Transitions Reference

```
WaitingForDeps → Pending    ✅
Pending → Queued             ✅
Queued → Proving             ✅
Proving → Completed          ✅
Proving → TransientFailure   ✅
TransientFailure → Queued    ✅ (retry)
Any → Failed                 ✅

Pending → Completed          ❌ INVALID
Pending → TransientFailure   ❌ INVALID
Queued → Completed           ❌ INVALID
Pending → Proving            ❌ INVALID
```

### Error Message Patterns

| Error Message | Cause | Fix |
|--------------|-------|-----|
| `invalid status transition from Pending to Completed` | Skipped Queued+Proving states | Add mark_queued(), mark_proving() calls |
| `invalid status transition from Queued to Completed` | Skipped Proving state | Add mark_proving() call |
| `invalid status transition from Pending to TransientFailure` | Skipped Queued+Proving states | Add state transitions before error handling |
| `Task not found` | Task ID doesn't exist | Check task creation, verify ID |

---

## Test Failures

### Prover Test Failures

#### 1. All Tests Timing Out

**Symptom:** 14/16 tests failing with timeout

**Root Cause:** State machine bug (fixed 2025-11-04)

**Solution:** Update to commit `baea23cf1` or later

#### 2. Specific Test Failure: prover_checkpoint_runner

**Status:** Known failure (unrelated to PaaS)

**Error:** Timeout waiting for epoch

**Root Cause:** Sequencer-side issue

**Workaround:** Skip this test, not a PaaS bug

#### 3. Specific Test Failure: prover_cl_dispatch

**Status:** Under investigation

**Next Steps:**
- Check CL state transition proof generation
- Verify CL operator initialization
- Review test expectations

#### 4. Specific Test Failure: prover_client_restart

**Status:** Under investigation

**Next Steps:**
- Check state persistence across restart
- Verify service cleanup and re-initialization
- Review test expectations

---

## Performance Issues

### Issue 1: Slow Proof Generation

**Expected Performance (Native backend):**
- Min: 7ms
- Max: 104ms
- Average: ~40ms

**If seeing >1s per proof:**
```bash
# Check backend type
grep "ProofZkVm" service.log

# SP1 proofs are much slower (expected)
# Native proofs should be fast

# Check system resources
top -p $(pgrep prover-client)
```

### Issue 2: High State Transition Overhead

**Expected Overhead:** ~377μs per task

**If seeing >10ms overhead:**
```bash
# Check timestamp deltas in logs
grep "marked as" service.log | head -20

# Look for delays between transitions
```

**Potential Causes:**
- Database contention
- Lock contention in TaskTracker
- Slow command channel

### Issue 3: Worker Pool Thrashing

**Symptoms:**
- Many tasks starting/stopping rapidly
- High CPU usage
- Tasks not completing

**Solution:**
```toml
# Reduce worker count
[workers]
worker_count = { Native = 2, SP1 = 1 }

# Increase polling interval
polling_interval_ms = 500
```

---

## Debugging Tools

### 1. Service Logs

**Location:**
```bash
# Functional tests
functional-tests/_dd/<test-id>/prover/prover_client/service.log

# Standalone binary
~/.strata/prover/logs/service.log
```

**Key Log Levels:**
- `ERROR` - Critical failures
- `WARN` - Transient failures, retries
- `INFO` - State transitions, major events
- `DEBUG` - Detailed execution flow

**Useful Grep Patterns:**
```bash
# All errors
grep "ERROR" service.log

# State transitions for specific task
grep "task_id=<id>" service.log

# Invalid transitions
grep "invalid status transition" service.log

# Task lifecycle
grep -E "(Starting proof|marked as|completed)" service.log

# Worker pool activity
grep -E "(Worker pool|worker limit)" service.log
```

### 2. Timing Analysis Script

**Location:** `/tmp/analyze_prover_timing.sh`

**Usage:**
```bash
bash /tmp/analyze_prover_timing.sh

# Output:
# Task: 7352fce3... Duration: 22ms
# Task: 8a93b4e1... Duration: 45ms
# ...
# Summary:
# Total tasks: 26
# Completed: 13
# Failed: 0
# Invalid transitions: 0
```

### 3. Health Report

**Location:** `/tmp/prover_health_report.md`

Comprehensive analysis of:
- State machine correctness ✅
- Error rates ✅
- Service initialization ✅
- Task performance ✅
- Code quality ✅

### 4. Runtime Metrics

**Via ProverHandle:**
```rust
// Get service status
let status = handle.status_rx().borrow().clone();
println!("Active: {}, Queued: {}, Completed: {}",
    status.active_tasks,
    status.queued_tasks,
    status.completed_tasks
);

// Get detailed report
let report = handle.get_report().await?;
println!("Worker utilization: {:.1}%",
    report.worker_utilization * 100.0
);
```

---

## Log Analysis

### Healthy Service Logs

```
2025-11-04T02:56:15.040783Z  INFO Worker pool started
2025-11-04T02:56:15.040884Z  INFO PaaS service launched

2025-11-04T02:56:17.179624Z  INFO Starting proof generation task_id=7352fce3...
2025-11-04T02:56:17.179739Z  INFO Task marked as queued task_id=7352fce3...
2025-11-04T02:56:17.179875Z  INFO Task marked as proving task_id=7352fce3...
2025-11-04T02:56:17.201926Z  INFO Proof generation completed task_id=7352fce3...
2025-11-04T02:56:17.202052Z  INFO Task marked as completed task_id=7352fce3...
```

**Key Indicators:**
- ✅ Service and worker pool started
- ✅ Task follows Pending → Queued → Proving → Completed
- ✅ No error messages
- ✅ Reasonable timing (7-104ms for Native)

### Unhealthy Service Logs

```
2025-11-04T02:50:15.123456Z  INFO Starting proof generation task_id=abc123...
2025-11-04T02:50:15.124567Z  ERROR Failed to mark task as completed
  e=Unexpected("invalid status transition from Pending to Completed")
```

**Red Flags:**
- ❌ Invalid state transition errors
- ❌ Tasks not progressing through states
- ❌ Excessive retry attempts
- ❌ Worker pool not starting
- ❌ Panic messages

### Task Lifecycle Patterns

**Pattern 1: Successful Task**
```
INFO Starting proof generation task_id=X
INFO Task marked as queued task_id=X          [+100μs]
INFO Task marked as proving task_id=X         [+100μs]
INFO Proof generation completed task_id=X     [+20-100ms]
INFO Task marked as completed task_id=X       [+100μs]
```

**Pattern 2: Transient Failure with Retry**
```
INFO Starting proof generation task_id=Y
INFO Task marked as queued task_id=Y
INFO Task marked as proving task_id=Y
ERROR Proof generation failed task_id=Y error="..."
WARN Task marked as transient failure task_id=Y
...
INFO Starting proof generation task_id=Y      [retry after backoff]
INFO Task marked as queued task_id=Y
INFO Task marked as proving task_id=Y
INFO Proof generation completed task_id=Y
INFO Task marked as completed task_id=Y
```

**Pattern 3: Permanent Failure**
```
INFO Starting proof generation task_id=Z
INFO Task marked as queued task_id=Z
INFO Task marked as proving task_id=Z
ERROR Proof generation failed task_id=Z error="..."
ERROR Task marked as failed task_id=Z         [after max retries]
```

---

## Quick Diagnostic Checklist

When encountering issues:

1. **Check Service Status**
   ```bash
   grep "PaaS service launched" service.log
   grep "Worker pool started" service.log
   ```

2. **Check for State Machine Errors**
   ```bash
   grep "invalid status transition" service.log
   ```

3. **Check Task Progression**
   ```bash
   grep "task_id=<id>" service.log | grep -E "(queued|proving|completed)"
   ```

4. **Check Error Messages**
   ```bash
   grep -E "(ERROR|WARN)" service.log | tail -20
   ```

5. **Check Worker Activity**
   ```bash
   grep "Starting proof generation" service.log | wc -l
   ```

6. **Check Configuration**
   ```bash
   cat config.toml | grep -A5 "workers"
   cat config.toml | grep -A5 "retry"
   ```

7. **Run Health Check**
   ```bash
   bash /tmp/analyze_prover_timing.sh
   ```

---

## Getting Help

If you can't resolve the issue:

1. **Gather Information:**
   - Service logs (last 100 lines)
   - Configuration file
   - Test output
   - Timing analysis results

2. **Check Documentation:**
   - [DESIGN.md](./DESIGN.md) - Architecture details
   - [TESTING.md](./TESTING.md) - Test procedures
   - [DEVELOPMENT.md](./DEVELOPMENT.md) - Development guide

3. **Search for Similar Issues:**
   - Check git history: `git log --grep="<keyword>"`
   - Check previous commits: `git log --oneline | grep paas`

4. **Create Detailed Bug Report:**
   - Describe symptoms
   - Include logs
   - Show reproduction steps
   - Note any configuration changes
