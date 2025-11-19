Implementation-Agnostic Perf Checker — Ideas

Goals
- Cross-binary coverage: measure Sequencer, Fullnode, EL (reth), Prover client without code changes.
- Implementation-agnostic: rely on public RPCs, logs, and process stats; avoid internal hooks.
- Scenario-driven: reuse functional envs to drive realistic workloads and failure modes.
- Actionable metrics: block time adherence, throughput, latency, backlog, resource headroom.
- Regressions visible in CI: machine-readable summaries, simple pass/fail gates.

What To Measure
- L2 block cadence: inter-block time distribution vs configured `rollup.block_time`.
- Tx latency: submit → included block; percentiles under increasing load.
- Throughput: tx/s, gas/s, blocks/s at steady state for defined scenarios.
- Backlog/queues: mempool size, unproven checkpoints, unrelayed items (via RPC or log parsing).
- Prover timings: checkpoint queued → proving started → proof produced → published.
- Catch-up speed: time to sync from snapshot or from genesis to head under target load.
- Resource usage (optional): process CPU, RSS, disk I/O for `strata-client`, `alpen-reth`, `strata-prover-client`, `bitcoind`.

Scenarios To Cover
- Baseline idle: zero txs; verify block cadence and low jitter.
- Light/medium/heavy: Locust-driven jobs at 10/100/N tps until near epoch gas limit.
- Burst traffic: short spikes that exceed steady rate; observe backlog drain time.
- Sync/catch-up: start a new fullnode while sequencer is at height H; measure time to parity.
- Failure/reorg: brief sequencer crash/restart; L1 reorg depth d; verify recovery time and no long stalls.

How To Integrate Here
- Reuse flexitest runtime (functional-tests/entry.py) and env configs (envs/testenv.py):
  - Add perf “group” tests under `functional-tests/tests/perf/` so they can be filtered via `--groups perf`.
  - Add a `PerfSession` helper in `functional-tests/perf/` that orchestrates monitors, load, and reporting.
  - Extend `RethLoadConfigBuilder` usage to parameterize spawn rates per scenario.
- Monitors (implementation-agnostic adapters):
  - BlockMonitor: subscribe/poll via EL RPC (reth HTTP) and Sequencer RPC for new L2 blocks; compute interarrival stats.
  - TxLatencyMonitor: submit txs and track `get_transaction_receipt`; compute p50/p90/p99.
  - ProverMonitor: poll prover client RPC for checkpoint states; derive stage durations and queue depth.
  - ResourceMonitor (optional): using `psutil` capture pid → cpu%, rss, io; gated behind availability.
  - BacklogMonitor: poll mempool size (if exposed) or estimate via tx submission gap and inclusion rate.
- Waiter extensions: wrap existing `RethWaiter`/`StrataWaiter` operations with timers for “time-to” metrics (no behavior change).
- Artifacts: write JSON + CSV into the run datadir (via `StrataRunContext.save_json_file`), e.g. `perf/summary.json`, `perf/block_times.csv`.

Data Model and Output
- Per-scenario summary (JSON):
  - scenario: name, params (spawn_rate, duration, block_time_target, epoch_gas_limit).
  - block_time: avg, p50, p90, p99, max, missed_intervals.
  - throughput: tx_s_avg, gas_s_avg, blocks_s_avg.
  - latency: submit_to_inclusion p50/p90/p99.
  - prover: queue_len_max, t_prove_avg, t_publish_avg, failures.
  - resources (optional): cpu%/rss p95 per binary.
- Time series (CSV): timestamps with block heights, mempool, queue sizes, cpu/mem, etc.
- Exit code contract: perf tests may assert simple SLO gates and fail on violation.

Assertions / Gates (configurable)
- Block cadence: p90(inter-block) <= 1.5 × target; max jitter < K seconds.
- Latency: p90 submit→include <= X seconds at load L.
- Throughput: sustained >= Y tx/s for Z seconds without backlog growth.
- Prover: proofs produced within T minutes; queue_len does not grow unbounded.

Adapters (keep it agnostic)
- RethAdapter: `eth_blockNumber`, `eth_getBlockByNumber`, receipts; optionally `txpool_status` if enabled.
- StrataAdapter: existing JSON-RPC (client status, recent blocks, checkpoints if exposed).
- ProverAdapter: dev RPCs already enabled by `ProverClientFactory` config.
- Fallback to log parsing when RPC not available; ensure parsers are tolerant and version-tagged.

Proposed Layout
- `functional-tests/perf/`
  - `session.py`: PerfSession (scenario orchestrator, reporters).
  - `monitors/`: block_monitor.py, tx_latency.py, prover_monitor.py, resources.py.
  - `adapters/`: reth.py, strata.py, prover.py.
  - `reporters/`: json_reporter.py, csv_reporter.py.
  - `scenarios.py`: constants and builders for baseline/light/heavy/burst.
  - `gates.py`: SLO checks.
- `functional-tests/tests/perf/`
  - `block_cadence.py`: verify block time under varying load.
  - `throughput_latency.py`: steady-state throughput and latencies.
  - `prover_timing.py`: checkpoint→proof→publish timing.
  - `sync_catchup.py`: follower catch-up time.

How Tests Would Look (sketch)
- Use existing envs:
  - `basic` for cadence/latency/throughput; configure `RethLoadConfigBuilder` with rate per scenario.
  - `state_diffs` or `prover` env for prover timing.
- Example flow:
  1) Start env via flexitest as today.
  2) Start `PerfSession` with selected monitors.
  3) Start load generator with desired `spawn_rate`.
  4) Observe for N seconds; collect stats.
  5) Stop load; run gates; write reports; assert.

Minimal Changes Needed In Factories
- Ensure services expose:
  - `pid` (from `ProcService`) for resource monitoring; and `datadir_path` (already present for some services).
  - RPC URLs are already attached (`create_rpc`, `create_web3`).
- Optionally add flags to enable metrics endpoints if available; otherwise rely on existing RPCs.

CI Integration
- Add a Just/Nix target: `just fun-tests --groups perf` with a shorter duration subset for CI.
- Store JSON summaries as CI artifacts; compare against previous baselines for regressions.
- Allow local “soak” runs by adjusting scenario durations via env vars.

Expected Benefits
- Early performance regressions detection in hot paths without invasive instrumentation.
- Capacity planning: quantify safe throughput and block-time adherence under realistic load.
- Repeatable baselines across machines and implementations.
- Shared vocabulary: consistent metrics and SLOs across teams.

Next Steps
- Scaffold `functional-tests/perf/` with `PerfSession`, BlockMonitor, JSON reporter.
- Add first perf test: `tests/perf/block_cadence.py` that validates block-time p90 under 0, 10, 50 tps.
- Wire outputs into runtime datadir; document how to run and interpret results in functional-tests/README.

