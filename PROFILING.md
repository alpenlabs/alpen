# Node Sync Profiling & Performance Monitoring

**Comprehensive profiling system for debugging slow node sync performance across distributed deployments.**

## Overview

Added CPU profiling (pprof-style) and Prometheus metrics for tracking:
- **DB writes**: Timing and payload sizes
- **Network/RPC calls**: Timing and payload sizes
- **CPU usage**: pprof flamegraphs
- **Block processing**: Per-stage timing (validation, execution, state transition, DB writes)

## Quick Start

### 1. Start strata-client with profiling enabled

```bash
cargo run --bin strata-client -- --config your-config.toml
```

The profiling server automatically starts on **port 6060**.

### 2. Access profiling endpoints

#### Prometheus Metrics
```bash
curl http://localhost:6060/metrics
```

#### CPU Profiling (30 seconds)
```bash
curl "http://localhost:6060/debug/pprof/profile?seconds=30" -o profile.pb.gz
```

## Endpoints

### Metrics Endpoint: `/metrics`
**Prometheus-compatible metrics for continuous monitoring**

```bash
# View all metrics
curl http://localhost:6060/metrics

# Common queries for Grafana/Prometheus
curl http://localhost:6060/metrics | grep block_processing
curl http://localhost:6060/metrics | grep rpc_call
curl http://localhost:6060/metrics | grep db_write
```

**Key Metrics:**
- `strata_block_processing_duration_seconds{stage="validation|execution|state_transition|db_write|total"}`
- `strata_blocks_processed_total{status="success|failed"}`
- `strata_db_write_duration_seconds{operation="put_block|put_chainstate"}`
- `strata_db_write_bytes{operation="put_block|put_chainstate"}`
- `strata_rpc_call_duration_seconds{endpoint="...",target="execution_engine|l2_sync_peer"}`
- `strata_rpc_payload_bytes{endpoint="...",direction="request|response",target="..."}`
- `strata_l2_block_fetch_duration_seconds{peer="rpc_peer"}`
- `strata_l2_blocks_fetched_total{peer="...",status="success|failed"}`

### CPU Profiling Endpoint: `/debug/pprof/profile`
**On-demand CPU flamegraphs in Google pprof format**

```bash
# Profile for 30 seconds (default)
curl "http://localhost:6060/debug/pprof/profile" -o profile.pb.gz

# Profile for custom duration
curl "http://localhost:6060/debug/pprof/profile?seconds=60" -o profile-60s.pb.gz
```

## Analyzing Performance

### 1. Real-time metrics monitoring

**Check current block processing breakdown:**
```bash
curl -s http://localhost:6060/metrics | grep 'strata_block_processing_duration_seconds'
```

**Example output:**
```
strata_block_processing_duration_seconds_bucket{stage="validation",le="0.001"} 245
strata_block_processing_duration_seconds_bucket{stage="execution",le="0.5"} 189
strata_block_processing_duration_seconds_bucket{stage="state_transition",le="0.05"} 234
strata_block_processing_duration_seconds_bucket{stage="db_write",le="0.1"} 198
strata_block_processing_duration_seconds_bucket{stage="total",le="1.0"} 167
```

**Identify bottlenecks:**
```bash
# Which stage takes the longest?
curl -s http://localhost:6060/metrics | grep 'strata_block_processing_duration_seconds_sum'

# How much data are we writing to DB?
curl -s http://localhost:6060/metrics | grep 'strata_db_write_bytes'

# RPC call latencies
curl -s http://localhost:6060/metrics | grep 'strata_rpc_call_duration_seconds'
```

### 2. CPU profiling with pprof

**Capture a profile:**
```bash
curl "http://localhost:6060/debug/pprof/profile?seconds=30" -o cpu.pb.gz
```

**Analyze with pprof tool:**
```bash
# Install pprof (Go tool)
go install github.com/google/pprof@latest

# View flamegraph in browser
pprof -http=:8080 cpu.pb.gz

# Command-line top functions
pprof -top cpu.pb.gz

# Generate SVG flamegraph
pprof -svg cpu.pb.gz > flamegraph.svg
```

**Flamegraph shows:**
- Which functions consume CPU
- Call stacks and hot paths
- Time spent in crypto, hashing, state transitions, serialization

## Grafana Dashboard Setup

1. **Add Prometheus data source** pointing to your strata-client nodes
2. **Configure scrape config** in `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'strata-node'
    static_configs:
      - targets:
          - 'node1:6060'
          - 'node2:6060'
          - 'node3:6060'
          - 'node4:6060'
          - 'node5:6060'
    scrape_interval: 15s
```

3. **Import example dashboard panels**:

```promql
# Block processing rate
rate(strata_blocks_processed_total{status="success"}[5m])

# Average block processing time
rate(strata_block_processing_duration_seconds_sum{stage="total"}[5m])
  / rate(strata_block_processing_duration_seconds_count{stage="total"}[5m])

# DB write latency (p95)
histogram_quantile(0.95, rate(strata_db_write_duration_seconds_bucket[5m]))

# RPC call latency by endpoint
rate(strata_rpc_call_duration_seconds_sum[5m])
  / rate(strata_rpc_call_duration_seconds_count[5m])

# Network bandwidth (bytes/sec)
rate(strata_rpc_payload_bytes_sum[5m])
```

## Troubleshooting Slow Sync

### Step 1: Identify the bottleneck

Run metrics query to see per-stage timing:
```bash
curl -s http://localhost:6060/metrics | \
  grep 'strata_block_processing_duration_seconds{stage=' | \
  grep -E '(sum|count)'
```

**If DB writes are slow:**
- Check disk I/O: `iostat -x 1`
- RocksDB compaction may be the issue
- Consider SSD vs HDD

**If execution is slow:**
- EL (execution layer) RPC latency
- Check `strata_rpc_call_duration_seconds{endpoint="new_payload_v2"}`
- Network issues to execution engine

**If state_transition is slow:**
- Pure CPU - crypto operations, hashing
- Use pprof to identify hot functions
- Check if block has many transactions

### Step 2: Compare nodes

When you have 5+ nodes, compare metrics across them:

```bash
# Node 1
curl -s http://node1:6060/metrics | grep 'block_processing_duration_seconds_sum{stage="total"}'

# Node 2
curl -s http://node2:6060/metrics | grep 'block_processing_duration_seconds_sum{stage="total"}'

# ...
```

If Node 2 is slower, check:
- Network latency to peers (L2 RPC)
- Network latency to execution engine
- CPU differences
- Disk performance differences

### Step 3: Deep dive with pprof

On the slow node, capture CPU profile:
```bash
curl "http://slow-node:6060/debug/pprof/profile?seconds=60" -o slow-node.pb.gz
pprof -http=:8080 slow-node.pb.gz
```

Look for:
- Functions consuming >10% CPU
- Unexpected hot paths (e.g., serialization taking 40%)
- Lock contention (sync primitives)

## Instrumented Code Paths

**Block Processing Pipeline:**
- `fork_choice_manager.rs:handle_new_block()` - Total block processing
  - Validation stage (signature checks)
  - Execution stage (EL payload submission)
  - State transition stage (CL STF)
  - DB write stage (chainstate + block data)

**Storage Layer:**
- `storage/managers/l2.rs:put_block_data_*()` - Block DB writes
- `storage/managers/chainstate.rs:put_write_batch_*()` - State DB writes

**RPC/Network:**
- `evmexec/http_client.rs` - EL RPC calls (new_payload, fork_choice_updated)
- `sync/client.rs:get_blocks()` - L2 block fetching from peers

## Port Configuration

**Default port:** `6060` (hardcoded in `main.rs:185`)

To change, edit:
```rust
// bin/strata-client/src/main.rs
profiling::start_profiling_server("0.0.0.0", YOUR_PORT),
```

For production, consider:
- Using separate port per environment
- Restricting to `127.0.0.1` if not exposing externally
- Adding authentication (not implemented)

## Kubernetes Deployment

Add annotation for Prometheus scraping:
```yaml
apiVersion: v1
kind: Pod
metadata:
  annotations:
    prometheus.io/scrape: "true"
    prometheus.io/port: "6060"
    prometheus.io/path: "/metrics"
spec:
  containers:
  - name: strata-client
    ports:
    - containerPort: 6060
      name: profiling
```

## Performance Baseline (Example)

**Healthy node (mainnet):**
- Block processing: ~200-500ms total
  - Validation: ~5-10ms
  - Execution: ~100-300ms (network to EL)
  - State transition: ~50-100ms (CPU)
  - DB write: ~20-50ms (disk I/O)
- DB writes: <100KB per block
- RPC latency to EL: <200ms
- L2 block fetch: <500ms for batch of 10 blocks

**Your numbers will vary** - use these as a starting point for comparison.

## Next Steps

1. **Start node with profiling** and verify endpoints work
2. **Run for a few minutes** to collect baseline metrics
3. **Check metrics endpoint** to see current performance
4. **Compare slow vs fast nodes** to identify differences
5. **Use pprof** to drill into CPU hotspots if needed

**For distributed debugging:** Set up Prometheus + Grafana to monitor all nodes in real-time.
