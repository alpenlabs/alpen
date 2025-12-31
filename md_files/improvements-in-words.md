# Logging & Observability Improvements: The Problem We're Solving

**Author:** Engineering Team
**Date:** 2025-12-03
**Purpose:** Explain why our current logging is broken and what we need to fix

---

## The Core Problem

When something goes wrong in production, you look at the logs and see a jumbled mess. Events from different operations are interleaved. You can't tell which log line belongs to which request. You spend hours trying to reconstruct what actually happened, often giving up and just restarting the service.

**This is unacceptable for a distributed system handling financial transactions.**

---

## What's Actually Broken

### Problem 1: No Request Correlation

**The Symptom:**
```
07:13:37.782  INFO  ASM found pivot anchor state pivot_block=100
07:13:37.801  INFO  Successfully reassigned expired assignments
07:13:44.958  WARN  tripped bail interrupt, exiting ctx=duty_sign_block
```

**Why It's Broken:**
You're looking at three log lines. The first two are from processing L1 block 100 and 101 respectively. The third is a system crash. But you can't tell:
- Which operation triggered the crash?
- What was happening before the crash?
- Were these events related or independent?

**The Fundamental Issue:**
Each log line is an isolated event. 
There's no thread connecting related events together. 
It's like trying to read a story where every sentence is shuffled randomly.

### Problem 2: Lost Context Across Service Boundaries

**The Symptom:**
Your sequencer client calls an RPC method on the sequencer. The RPC call fails. You see:
- Client side: "RPC call failed"
- Server side: "Request processed successfully"

**Why It's Broken:**
The client and server logs are completely disconnected. You can't prove they're talking about the same request. Was the failure a network issue? Did the server actually fail but not log it? You'll never know.

**The Fundamental Issue:**
When a request crosses a service boundary via RPC, we lose the ability to trace it. Each service thinks it's handling an independent operation.

### Problem 3: Abbreviated Identifiers Are Useless

**The Symptom:**
```
INFO  processing block blkid=aa026e..91422d
```

**Why It's Broken:**
Try to grep for that block ID. You can't. The `..` breaks your grep pattern. You have the full ID somewhere, but you abbreviated it "for readability". Now you can't correlate logs about the same block.

**The Fundamental Issue:**
Logs optimized for human reading at the expense of machine greppability are useless logs. Logs exist to be filtered, correlated, and aggregated. Pretty formatting is secondary.

### Problem 4: Actor Model Makes Everything Worse

**The Symptom:**
You have 10 async worker tasks all processing different messages concurrently.
Each one logs "processing message". 
Your log file has 1,000 "processing message" entries interleaved randomly. Good luck figuring out which 100 belong to the request you care about.

**Why It's Broken:**
Traditional single-threaded logging assumes sequential execution. You log line A, then B, then C, and they appear in that order. With actors and async tasks, you log A1, B1, A2, C1, B2, A3... The timeline is destroyed.

**The Fundamental Issue:**
Async + actors means you need **explicit correlation** because implicit ordering (timestamps) is worthless.

### Problem 5: No Semantic Filtering

**The Symptom:**
You want to see "everything the fork choice manager did".
You grep for `fork_choice_manager`.
You get 1,000 lines.
But you also miss 500 lines because some logs say `fcm`,
some say `consensus`,
some say `chain_worker` (which calls fork choice manager).

**Why It's Broken:**
Module paths (like `strata_consensus_logic::fork_choice_manager`) are **structural**, not **semantic**:
- They tell you the **file/module** where code lives (useful for debugging)
- They DON'T tell you the **logical component** it belongs to (needed for filtering)
- They change when you refactor (rename files, move modules, break your grep)
- One logical component might span multiple modules (can't grep for all of them)
- Multiple modules might be part of one logical component (grep gets noise)

**Example:**
```
strata_consensus_logic::fork_choice_manager::process_block   ← Module path
strata_consensus_logic::unfinalized_tracker::update_chain    ← Different module path
strata_consensus_logic::chain_tracker::add_block            ← Different module path
```

All three are part of the "fork choice manager" **conceptual component**, but have different module paths. Grep for `fork_choice_manager` misses the other two!

**The Fundamental Issue:**
Logs need **explicit semantic tags** (like `component=fork_choice_manager`) that:
- Represent logical components, not code structure
- Survive refactoring (rename files without breaking queries)
- Are consistent across multiple modules that are part of one component
- Can be set explicitly on spans instead of relying on automatic module paths

---

## What We Need to Fix

### Fix 1: Request IDs That Flow Everywhere

**The Concept:**
Every logical operation gets a unique ID at its entry point. This ID is attached to every log line, every span, every child operation. When you grep for that ID, you see the complete story of that operation from start to finish, across all services.

**What Changes:**
- L1 block arrives → generate request ID → attach to all logs while processing that block
- RPC call arrives → extract request ID from caller → attach to all server-side logs
- Spawn a task → propagate request ID to child task

**The Result:**
```bash
$ grep "req_id=a1b2c3d4" service.log
# Shows the ENTIRE lifecycle of request a1b2c3d4, in chronological order
```

No matter how many services it touched, no matter how many async tasks it spawned, you see everything.

### Fix 2: Distributed Trace Propagation

**The Concept:**
When service A calls service B via RPC, the trace context flows with the request. Service B doesn't create a new isolated operation - it continues the trace that service A started.

**What Changes:**
- RPC client: Inject trace context into request metadata
- RPC server: Extract trace context from request metadata
- Logs on server include the same request ID as logs on client

**The Result:**
You can see a request flow through:
1. Sequencer client receives duty
2. Sequencer client calls `getBlockTemplate` RPC
3. Sequencer server processes template request
4. Sequencer server queries database
5. Sequencer server returns template
6. Sequencer client signs and submits

All logs have the same `req_id`. You can follow the entire flow through both services.

### Fix 3: Full Identifiers, Always

**The Concept:**
Block IDs, transaction hashes, epoch numbers - always log the complete identifier. Never abbreviate. Storage is cheap. Your time debugging is expensive.

**What Changes:**
- Stop: `blkid=aa026e..91422d`
- Start: `l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d`

**The Result:**
```bash
$ grep "l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d" *.log
# Shows every log line about this specific block
```

You can grep. You can correlate. You can trace.

### Fix 4: Component Tags on Everything

**The Concept:**
Every span, every log line, gets a `component` field that identifies which subsystem produced it. This is a semantic tag, not a file path. It survives refactoring.

**What Changes:**
Every span must include:
```
component = "fork_choice_manager"   // or "asm_worker", "rpc_client", etc.
```

**The Result:**
```bash
$ grep "component=fork_choice_manager" service.log
# Shows ONLY fork choice manager activity, nothing else
```

You can filter by conceptual subsystem, not by file path.

### Fix 5: Structured Logging Everywhere

**The Concept:**
Stop logging messages. Start logging structured events with typed fields. Logs become queryable data, not text to parse with regex.

**What Changes:**
- Stop: `error!("failed to process block 100")`
- Start: `error!(l1_height = 100, l1_block = %blkid, component = "asm_worker", "block processing failed")`

**The Result:**
In your log aggregation system (Loki, Datadog, etc.):
```
query: {component="asm_worker"} | json | l1_height > 100
```

You can filter, aggregate, and analyze logs like a database.

---

## Why This Matters

### Scenario: Production Crash

**Before (Current State):**
1. Service crashes at 3am
2. You wake up, look at logs
3. See 100,000 lines of jumbled text
4. Spend 2 hours trying to figure out what happened
5. Give up, restart service, pray it doesn't happen again
6. It happens again next week

**After (With Fixes):**
1. Service crashes at 3am
2. Alert includes request ID: `req_id=f7e3a1c2`
3. You grep: `grep "req_id=f7e3a1c2" service.log`
4. See complete timeline of that specific request
5. Identify root cause in 5 minutes
6. Fix bug, deploy, problem solved

### Scenario: Slow Request

**Before:**
1. User complains "block processing is slow"
2. You look at logs, see thousands of "processing block" messages
3. Can't tell which ones are slow
4. Can't tell where time is spent
5. Add more debug logs, redeploy, hope to catch it next time

**After:**
1. OpenTelemetry records span durations automatically
2. Grafana shows: "Request f7e3a1c2 took 5 seconds, 4.8s in database query"
3. You immediately know: database is slow, not our code
4. Optimize query, problem solved

### Scenario: Cross-Service Bug

**Before:**
1. Sequencer client says: "RPC call failed"
2. Sequencer server says: "All requests succeeded"
3. You have no way to prove they're talking about the same request
4. Is it network? Is it server-side bug? You don't know
5. Spend days trying to reproduce

**After:**
1. Sequencer client logs: `req_id=abc123, rpc_method=getBlockTemplate, status=failed`
2. Sequencer server logs: `req_id=abc123, rpc_method=getBlockTemplate, status=success`
3. You immediately see: Server thinks it succeeded, client thinks it failed
4. Obvious conclusion: Response was corrupted/lost in transit
5. Add retries with idempotency keys, problem solved

---

## The Mental Model You Need

### Think In Terms of Logical Operations, Not Code Paths

**Wrong Mental Model:**
"This function processes blocks. I'll add a log at the start and end of the function."

**Right Mental Model:**
"A logical operation is: processing L1 block 100. That operation might:
- Start in the L1 reader task
- Cross into the ASM worker
- Trigger an RPC call to the database
- Spawn a child task to validate
- End in the fork choice manager

All of these are part of ONE logical operation and should share ONE request ID."

### Think In Terms of Observability, Not Debugging

**Wrong Mental Model:**
"I'll add logs when I'm debugging, remove them when done."

**Right Mental Model:**
"Logs are permanent instrumentation. They exist in production. They're how we understand system behavior, detect issues, and measure performance. They're not debug printf statements."

### Think In Terms of Queries, Not Grep

**Wrong Mental Model:**
"I'll grep for 'error' and see what broke."

**Right Mental Model:**
"I'll query: Show me all errors from the fork choice manager in the last hour where L1 height > 1000. Then group by error type and show me the most common failure."

Logs are data. Treat them like data.

---

## What Good Instrumentation Looks Like

### Property 1: Request Correlation

Pick any request. You can see its complete lifecycle from entry to exit, across all services, in chronological order.

### Property 2: Semantic Filtering

You can filter logs by:
- Component/subsystem (fork_choice_manager, asm_worker, rpc_server)
- Operation type (block processing, RPC call, state transition)
- Entity (specific block, transaction, epoch)
- Status (success, error, timeout)

Without knowing file paths or function names.

### Property 3: Causality Preservation

When operation A spawns operation B, you can see that relationship. Parent-child spans make the dependency graph explicit.

### Property 4: Performance Visibility

You can see:
- How long each operation took
- Where time was spent (network, CPU, database)
- Which operations are slow outliers
- Trends over time

Without manual instrumentation or profiling.

### Property 5: Context on Error

When something fails, you automatically get:
- What operation was being attempted
- What entity was being processed
- What the system state was
- What the error chain was
- Who/what triggered the operation

No manual error message crafting needed.

---

## Common Objections

### "This adds too much overhead"

**Reality:** Structured logging adds 1-3% CPU overhead. Span creation costs ~100 bytes of memory per operation. This is negligible compared to the cost of production outages caused by poor observability.

**Trade-off:** Would you rather spend 3% more CPU all the time, or lose 100% of revenue when you can't debug a production issue?

### "Logs will be too big"

**Reality:** With rate limiting on hot paths and sampling on high-frequency operations, log volume increases 2-3x, not 10x. And you can grep/filter it, so you're not looking at all of it anyway.

**Trade-off:** Storage costs $0.02/GB. Your time costs $100/hour. Store more logs.

### "It's too much boilerplate"

**Reality:** Creating a span is 3 lines of code. Logging an error with context is 5 lines. This is not meaningful overhead.

**Trade-off:** Write 5 lines now, or spend 5 hours debugging later?

### "We can add it later when we need it"

**Reality:** You need it now. You're already spending hours debugging issues that would be trivial with proper instrumentation.

**False Economy:** "We'll add observability when we have time" means "We'll add it after the third production outage where we can't figure out what went wrong."

---

## What Success Looks Like

### Metric 1: Time to Root Cause

**Before:** Hours or days to understand production issues.

**After:** Minutes to identify root cause from logs/traces.

### Metric 2: Reproducibility

**Before:** "I can't reproduce the issue" means we give up.

**After:** Logs contain enough context to understand what happened even if we can't reproduce it.

### Metric 3: Cross-Team Debugging

**Before:** "Is this a client bug or server bug?" becomes a blame game.

**After:** Traces show exactly where the failure occurred and why.

### Metric 4: Performance Optimization

**Before:** Guess at what's slow, add profiling, redeploy, hope to catch it.

**After:** Traces show exactly which operations are slow and where time is spent.

### Metric 5: Confidence in Production

**Before:** Scared to deploy because debugging production issues is hard.

**After:** Confident to deploy because you can quickly understand and fix issues.

---

## The Path Forward

### Phase 1: Add Correlation IDs

Start attaching request IDs to logs. Even without distributed tracing, this makes logs filterable by operation.

### Phase 2: Add Component Tags

Tag every span with its component. Make logs filterable by subsystem.

### Phase 3: Add Distributed Tracing

Propagate trace context across RPC boundaries. Make cross-service operations visible.

### Phase 4: Fix Anti-Patterns

Stop abbreviating IDs. Add rate limiting to hot paths. Use structured fields consistently.

### Phase 5: Operationalize

Set up Grafana dashboards. Create runbooks for trace-based debugging. Train team on new tools.

---

## Key Principles to Remember

### Principle 1: Logs Are Permanent Instrumentation

Don't think of logs as temporary debug output. They're permanent monitoring infrastructure. Invest in them accordingly.

### Principle 2: Context Is Everything

A log line without context is noise. A log line with full context (request ID, entity IDs, component, error chain) is actionable information.

### Principle 3: Correlation Is Non-Negotiable

In a distributed async system, if you can't correlate events, you can't understand behavior. Request IDs aren't optional.

### Principle 4: Structured > Unstructured

Messages for humans. Structured fields for machines. Logs need to be machine-queryable.

### Principle 5: Observability Is a Feature

Treat observability like any other system requirement. It has costs (performance, code complexity) and benefits (debuggability, reliability). The ROI is positive.

---

## Questions to Ask Yourself

When adding any logging:

1. **Can I correlate this log with related events?**
   - Does it have a request ID?
   - Does it have a component tag?

2. **Can I filter for this specific case?**
   - Are entity IDs full and grepable?
   - Are field names consistent?

3. **Will this log be useful in 6 months?**
   - Does it have enough context?
   - Will I know what it means without reading code?

4. **Am I creating noise?**
   - Is this in a hot loop?
   - Should this be rate-limited?

5. **Does this propagate context?**
   - If this spawns a task, does the task get the request ID?
   - If this makes an RPC call, does the call carry trace context?

---

## The Bottom Line

**Current State:** Logs are a jumbled mess. Debugging is painful. Production issues are scary.

**Desired State:** Logs are structured, correlated, and queryable. Debugging is systematic. Production issues are manageable.

**What It Takes:**
- Request IDs everywhere
- Component tags on all spans
- Full identifiers in logs
- Distributed trace propagation
- Rate limiting on hot paths

**What You Get:**
- 10x faster debugging
- Cross-service visibility
- Performance insights
- Confidence in production
- Fewer 3am wakeup calls

**The Trade-Off:**
- 1-3% performance overhead
- 5 lines of boilerplate per operation
- 2-3x log volume

**The ROI:**
Your time is worth more than CPU cycles. Your sleep is worth more than disk space. Invest in observability.

---

## Next Steps

1. Read this document
2. Read `standards-to-follow.md` for implementation details
3. Read `improvements.md` for code examples (if needed)
4. Start with your next feature: add request IDs and component tags
5. Gradually adopt the patterns in existing code
6. Celebrate when debugging gets easier

**Remember:** Good observability isn't about logging everything. It's about logging the right things in a way that lets you reconstruct what happened when something goes wrong. Every log line should answer the question: "What was the system doing and why?"
