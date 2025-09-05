# Variables
git_tag := `git describe --tags --abbrev=0 2>/dev/null || echo "no-tag"`
build_path := "target"
functional_tests_dir := "functional-tests"
functional_tests_datadir := "_dd"
docker_dir := "docker"
docker_datadir := ".data"
prover_perf_eval_dir := "bin/prover-perf"
prover_proofs_cache_dir := "provers/tests/proofs"
prover_programs := "btc-blockscan,evm-ee-stf,cl-stf,checkpoint"
profile := env("PROFILE", "release")
cargo_install_extra_flags := env("CARGO_INSTALL_EXTRA_FLAGS", "")
features := env("FEATURES", "")
docker_image_name := env("DOCKER_IMAGE_NAME", "")
unit_test_args := "--locked --workspace -E 'kind(lib)' -E 'kind(bin)' -E 'kind(proc-macro)'"
cov_file := "lcov.info"

# Default recipe - show available commands
default:
    @just --list

# Build the workspace into the `target` directory
[group('build')]
build:
    cargo build --workspace --bin "strata-client" --features "{{features}}" --profile "{{profile}}"

# Run unit tests
[group('test')]
test-unit: ensure-cargo-nextest
    cargo nextest run {{unit_test_args}}

# Run unit tests with coverage
[group('test')]
cov-unit: ensure-cargo-llvm-cov ensure-cargo-nextest
    rm -f {{cov_file}}
    cargo llvm-cov nextest --lcov --output-path {{cov_file}} {{unit_test_args}}

# Generate an HTML coverage report and open it in the browser
[group('test')]
cov-report-html: ensure-cargo-llvm-cov ensure-cargo-nextest
    cargo llvm-cov --open --workspace --locked nextest

# Run integration tests
[group('test')]
test-int: ensure-cargo-nextest
    cargo nextest run -p "integration-tests" --status-level=fail --no-capture

# Runs `nextest` under `cargo-mutants`. Caution: This can take *really* long to run
[group('test')]
mutants-test: ensure-cargo-mutants
    cargo mutants --workspace -j2

# Check for security advisories on any dependencies
[group('test')]
sec: ensure-cargo-audit
    cargo audit

# Generate reports and profiling data for proofs
[group('prover')]
prover-eval: prover-clean
    cd {{prover_perf_eval_dir}} && RUST_LOG=info ZKVM_MOCK=1 ZKVM_PROFILING=1 cargo run --release -- --programs {{prover_programs}}

# Cleans up proofs and profiling data generated
[group('prover')]
prover-clean:
    rm -rf {{prover_perf_eval_dir}}/*.trace
    rm -rf {{prover_proofs_cache_dir}}/*.proof

# Check if cargo-audit is installed
[group('prerequisites')]
ensure-cargo-audit:
    #!/usr/bin/env bash
    if ! command -v cargo-audit &> /dev/null;
    then
        echo "cargo-audit not found. Please install it by running the command 'cargo install cargo-audit'"
        exit 1
    fi

# Check if cargo-llvm-cov is installed
[group('prerequisites')]
ensure-cargo-llvm-cov:
    #!/usr/bin/env bash
    if ! command -v cargo-llvm-cov &> /dev/null;
    then
        echo "cargo-llvm-cov not found. Please install it by running the command 'cargo install cargo-llvm-cov --locked'"
        exit 1
    fi

# Check if cargo-mutants is installed
[group('prerequisites')]
ensure-cargo-mutants:
    #!/usr/bin/env bash
    if ! command -v cargo-mutants &> /dev/null;
    then
        echo "cargo-mutants not found. Please install it by running the command 'cargo install cargo-mutants'"
        exit 1
    fi

# Check if cargo-nextest is installed
[group('prerequisites')]
ensure-cargo-nextest:
    #!/usr/bin/env bash
    if ! command -v cargo-nextest &> /dev/null;
    then
        echo "cargo-nextest not found. Please install it by running the command 'cargo install cargo-nextest --locked'"
        exit 1
    fi

# Check if codespell is installed
[group('prerequisites')]
ensure-codespell:
    #!/usr/bin/env bash
    if ! command -v codespell &> /dev/null;
    then
        echo "codespell not found. Please install it by running the command 'pip install codespell' or refer to the following link for more information: https://github.com/codespell-project/codespell"
        exit 1
    fi

# Check if uv is installed
[group('prerequisites')]
ensure-uv:
    #!/usr/bin/env bash
    if ! command -v uv &> /dev/null;
    then
        echo "uv not found. Please install it by following the instructions from: https://docs.astral.sh/uv/"
        exit 1
    fi

# Check if taplo is installed
[group('prerequisites')]
ensure-taplo:
    #!/usr/bin/env bash
    if ! command -v taplo &> /dev/null;
    then
        echo "taplo not found. Please install it by following the instructions from: https://taplo.tamasfe.dev/cli/installation/binary.html"
        exit 1
    fi

# Activate uv environment for integration tests
[group('functional-tests')]
activate-uv: ensure-uv
    cd {{functional_tests_dir}} && uv venv --clear
    @if [ -n "${FISH_VERSION:-}" ]; then source {{functional_tests_dir}}/.venv/bin/activate.fish; else source {{functional_tests_dir}}/.venv/bin/activate; fi

# Remove the data directory used by functional tests
[group('functional-tests')]
clean-dd:
    rm -rf {{functional_tests_dir}}/{{functional_tests_datadir}} 2>/dev/null || true

# cargo clean
[group('functional-tests')]
clean-cargo:
    cargo clean 2>/dev/null || true

# Remove docker data files inside /docker/.data
[group('functional-tests')]
clean-docker-data:
    rm -rf {{docker_dir}}/{{docker_datadir}} 2>/dev/null || true

# Remove uv virtual environment
[group('functional-tests')]
clean-uv:
    cd {{functional_tests_dir}} && rm -rf .venv 2>/dev/null || true

# Clean functional tests directory, cargo clean, clean docker data, clean uv virtual environment
[group('functional-tests')]
clean: clean-dd clean-docker-data clean-cargo clean-uv
    @echo "\n\033[36m======== CLEAN_COMPLETE ========\033[0m\n"

# Docker compose up
[group('functional-tests')]
docker-up:
    cd {{docker_dir}} && docker compose up -d

# Docker compose down
[group('functional-tests')]
docker-down:
    cd {{docker_dir}} && docker compose down && rm -rf {{docker_datadir}} 2>/dev/null || true

# Runs functional tests
[group('functional-tests')]
test-functional: ensure-uv activate-uv clean-dd
    cd {{functional_tests_dir}} && ./run_test.sh

# Check formatting issues but do not fix automatically
[group('code-quality')]
fmt-check-ws:
    cargo fmt --check

# Format source code in the workspace
[group('code-quality')]
fmt-ws:
    cargo fmt --all

# Runs `taplo` to check that TOML files are properly formatted
[group('code-quality')]
fmt-check-toml: ensure-taplo
    taplo fmt --check

# Runs `taplo` to format TOML files
[group('code-quality')]
fmt-toml: ensure-taplo
    taplo fmt

# Check formatting of python files inside `test` directory
[group('code-quality')]
fmt-check-func-tests: ensure-uv activate-uv
    cd {{functional_tests_dir}} && uv run ruff format --check

# Apply formatting of python files inside `test` directory
[group('code-quality')]
fmt-func-tests: ensure-uv activate-uv
    cd {{functional_tests_dir}} && uv run ruff format

# Checks for lint issues in the workspace
[group('code-quality')]
lint-check-ws:
    cargo clippy \
        --workspace \
        --bin "strata-client" \
        --lib \
        --examples \
        --tests \
        --benches \
        --all-features \
        --no-deps \
        -- -D warnings

# Lints the workspace and applies fixes where possible
[group('code-quality')]
lint-fix-ws:
    cargo clippy \
        --workspace \
        --bin "strata-client" \
        --lib \
        --examples \
        --tests \
        --benches \
        --all-features \
        --fix \
        --no-deps \
        --allow-dirty \
        -- -D warnings

# Runs `codespell` to check for spelling errors
[group('code-quality')]
lint-check-codespell: ensure-codespell
    codespell

# Runs `codespell` to fix spelling errors if possible
[group('code-quality')]
lint-fix-codespell: ensure-codespell
    codespell -w

# Lints TOML files
[group('code-quality')]
lint-check-toml: ensure-taplo
    taplo lint

# Lints the functional tests
[group('code-quality')]
lint-check-func-tests: ensure-uv activate-uv
    cd {{functional_tests_dir}} && uv run ruff check

# Lints the functional tests and applies fixes where possible
[group('code-quality')]
lint-fix-func-tests: ensure-uv activate-uv
    cd {{functional_tests_dir}} && uv run ruff check --fix

# Runs all lints and checks for issues without trying to fix them
[group('code-quality')]
lint: fmt-check-ws fmt-check-func-tests fmt-check-toml lint-check-ws lint-check-func-tests lint-check-codespell
    @echo "\n\033[36m======== OK: Lints and Formatting ========\033[0m\n"

# Runs all lints and applies fixes where possible
[group('code-quality')]
lint-fix: fmt-toml fmt-ws lint-fix-ws lint-fix-codespell
    @echo "\n\033[36m======== OK: Lints and Formatting Fixes ========\033[0m\n"

# Runs `cargo docs` to generate the Rust documents in the `target/doc` directory
[group('code-quality')]
rustdocs:
    RUSTDOCFLAGS="\
    --show-type-layout \
    --enable-index-page -Z unstable-options \
    -A rustdoc::private-doc-tests \
    -D warnings" \
    cargo doc \
    --workspace \
    --no-deps

# Runs doctests on the workspace
[group('code-quality')]
test-doc:
    cargo test --doc --workspace

# Runs all tests in the workspace including unit and docs tests
[group('code-quality')]
test: test-unit test-doc

# Runs lints (without fixing), audit, docs, and tests (run this before creating a PR)
[group('code-quality')]
pr: lint rustdocs test-doc test-unit test-int test-functional
    @echo "\n\033[36m======== CHECKS_COMPLETE ========\033[0m\n"
    @test -z \`git status --porcelain\` || echo "WARNING: You have uncommitted changes"
    @echo "All good to create a PR!"

# Docker restart (down then up)
[group('functional-tests')]
docker: docker-down docker-up
    echo "Done!"
