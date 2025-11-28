Perf Integration and Flamegraphs — Ideas and Plan

Objectives
- Capture CPU profiles of core binaries (`strata-client`, `alpen-reth`, `strata-prover-client`) in realistic scenarios.
- Use Linux `perf` (or `cargo-flamegraph`) to produce flamegraphs and numeric summaries for regression detection.
- Integrate short captures into GitHub Actions CI and publish SVG/summary artifacts.
- Keep it implementation-agnostic: no code changes inside binaries; rely on public flags and OS tooling.

Approach Overview
- Scenarios: run existing functional envs (e.g., `basic`, `load_reth`) to exercise hot paths.
- Attach profiler: sample a selected process PID for N seconds at 99–199 Hz.
- Generate outputs: `perf.data`, collapsed stacks, `flamegraph.svg`, and top-N symbol table with percentages.
- CI gating: use `perf stat` (low overhead) for numeric gates; keep flamegraphs as visual artifacts.

Local Setup (Linux)
- Install perf and FlameGraph utilities:
  - Ubuntu: `sudo apt-get update && sudo apt-get install -y linux-tools-common linux-tools-generic linux-tools-$(uname -r)`
  - FlameGraph scripts: `git clone https://github.com/brendangregg/FlameGraph` or use `cargo install flamegraph` (wraps perf + inferno)
- Enable perf access if needed:
  - `sudo sysctl kernel.perf_event_paranoid=-1`
  - `sudo sysctl kernel.kptr_restrict=0`

Profiling Commands (attach to a PID)
- Find PIDs (examples):
  - `pgrep -f strata-client`, `pgrep -f alpen-reth`, `pgrep -f strata-prover-client`
- Record 30s at 99 Hz with call-graph (DWARF is safest for Rust):
  - `sudo perf record -F 99 -g --call-graph dwarf -p $PID -- sleep 30`
- Generate collapsed stacks and flamegraph (FlameGraph repo in `$FLAME`):
  - `sudo perf script | $FLAME/stackcollapse-perf.pl > out.folded`
  - `$FLAME/flamegraph.pl --title "strata-client (30s @99Hz)" out.folded > flamegraph.svg`
- Rust demangling: add `--demangle` to `perf script` or use `inferno-flamegraph` via `cargo flamegraph`.

Alternative: cargo-flamegraph (runs the binary)
- For dev binaries or benches (not pid attach): `cargo flamegraph --root --bin strata-client -- -your-args`
- Caveat: needs `perf`, higher overhead, and may not reflect systemd/service launch wrappers.

Numeric Regression Signals
- Use `perf stat` for a quick, low-overhead numeric snapshot on a stable micro-scenario:
  - `sudo perf stat -e task-clock,cycles,instructions,branches,branch-misses,L1-dcache-loads,L1-dcache-load-misses -p $PID -- sleep 15`
- Parse the summary to derive:
  - `IPC = instructions/cycles`
  - branch-miss %, L1 miss %, task-clock (CPU time), context-switches.
- Gate examples (tune per scenario):
  - IPC not lower than baseline by >10%.
  - Branch-miss% not worse than baseline by >20%.
  - Task-clock (CPU time) for same workload not +25%.

Scenario Integration (with functional-tests)
- Start env via `functional-tests/entry.py` and the desired group:
  - Baseline cadence: `python3 functional-tests/entry.py --groups perf --env basic`
  - Load: `python3 functional-tests/entry.py --groups perf --env load_reth`
- Attach perf to the target PID for a bounded duration (e.g., 30–60s) during steady state.
- Store outputs in the run datadir (e.g., `functional-tests/_dd/<run>/perf/<svc>/flamegraph.svg`).
- Optional: add a small helper that queries PIDs from `flexitest` ProcService to avoid `pgrep`.

GitHub Actions (Linux) — Short Capture Example
- Notes: GitHub Ubuntu runners allow `sudo`; `perf` is available after tools install. Keep capture short (<30s) to control time.

Example workflow (sketch):

  name: perf-smoke
  on:
    pull_request:
      paths:
        - 'crates/**'
        - 'functional-tests/**'
        - 'Cargo.*'
        - 'perf_ideas.md'
  jobs:
    linux-perf:
      runs-on: ubuntu-22.04
      timeout-minutes: 25
      steps:
        - uses: actions/checkout@v4
        - name: Setup Rust
          uses: dtolnay/rust-toolchain@stable
        - name: Install perf tools
          run: |
            sudo apt-get update
            sudo apt-get install -y linux-tools-common linux-tools-generic linux-tools-$(uname -r)
            git clone --depth 1 https://github.com/brendangregg/FlameGraph $GITHUB_WORKSPACE/FlameGraph
            echo kernel.perf_event_paranoid=-1 | sudo tee /etc/sysctl.d/99-perf.conf
            echo kernel.kptr_restrict=0 | sudo tee -a /etc/sysctl.d/99-perf.conf
            sudo sysctl --system
        - name: Build workspace (release)
          run: cargo build --workspace --release
        - name: Start minimal env
          run: |
            # run a short env in background (adjust as needed)
            python3 functional-tests/entry.py --env basic &
            echo $! > env.pid
            sleep 15  # warmup
        - name: Capture perf (strata-client)
          run: |
            set -euo pipefail
            FLAME=$GITHUB_WORKSPACE/FlameGraph
            PID=$(pgrep -f "strata-client" | head -n1)
            sudo perf record -F 99 -g --call-graph dwarf -p "$PID" -- sleep 20
            sudo perf script --demangle | $FLAME/stackcollapse-perf.pl > strata.folded
            $FLAME/flamegraph.pl --title "strata-client (CI @99Hz/20s)" strata.folded > strata-flame.svg
        - name: Capture perf stat (sanity numbers)
          run: |
            PID=$(pgrep -f "strata-client" | head -n1)
            sudo perf stat -e task-clock,cycles,instructions,branches,branch-misses -p "$PID" -- sleep 10 2> perf-stat.txt || true
        - name: Upload artifacts
          uses: actions/upload-artifact@v4
          with:
            name: perf-artifacts
            path: |
              strata-flame.svg
              perf.data
              perf-stat.txt
        - name: Teardown env
          if: always()
          run: |
            kill $(cat env.pid) || true
            pkill -f strata-client || true

Regression Detection Strategy
- Keep flamegraphs as artifacts for visual diff; use `perf stat` for simple numeric gating.
- Optional deeper gate: top-N symbols self-time comparison
  - `perf report --stdio --sort symbol --percent-limit 1 > perf-report.txt`
  - Script parses percent per symbol and compares to a baseline JSON in repo (tolerances ±X%).
  - Store/update baseline per scenario in `perf/baselines/*.json`.

Platform Notes
- Linux: preferred, uses `perf`. Overhead is manageable with 99 Hz and short durations.
- macOS: use `dtrace`/`sample`/`Instruments` or `cargo-instruments`; harder to automate in CI.
- Containers: `perf` needs `CAP_PERFMON` or `--privileged`; GitHub hosted Ubuntu runners run host-level.

Best Practices
- Use `--call-graph dwarf` for Rust stack correctness (acceptable overhead for short captures).
- Keep captures short (10–30s) and targeted to steady-state window after warmup.
- Build with `--release` and include debug symbols (`[profile.release] debug = 1`) to retain symbol names.
- Set `RUSTFLAGS="-C force-frame-pointers=yes"` if using frame-pointer unwinding.

Next Steps
- Add a helper script `bin/perf_capture.sh` to encapsulate PID lookup, perf record, and flamegraph generation.
- Add a `perf-smoke` GitHub Actions job using the snippet above.
- Optionally add a small parser to convert `perf report --stdio` into JSON and gate against baselines.

