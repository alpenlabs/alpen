#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: assert-eest-prover-pipeline.sh [options]

Options:
  --baseline-file FILE     File containing byte offset captured before EEST.
  --log-file FILE          Alpen-client service.log to inspect.
  --docker-container NAME  Docker container whose logs should be inspected.
  --timeout SEC            Max wait for proof signals. Default: 900.
  --poll SEC               Poll interval. Default: 5.
  -h, --help               Show this help.
EOF
}

BASELINE_FILE=""
LOG_FILE=""
DOCKER_CONTAINER=""
TIMEOUT_SECONDS="900"
POLL_SECONDS="5"

while (($#)); do
    case "$1" in
        --baseline-file)
            BASELINE_FILE="${2:?missing value for --baseline-file}"
            shift 2
            ;;
        --log-file)
            LOG_FILE="${2:?missing value for --log-file}"
            shift 2
            ;;
        --docker-container)
            DOCKER_CONTAINER="${2:?missing value for --docker-container}"
            shift 2
            ;;
        --timeout)
            TIMEOUT_SECONDS="${2:?missing value for --timeout}"
            shift 2
            ;;
        --poll)
            POLL_SECONDS="${2:?missing value for --poll}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

if [[ -z "${BASELINE_FILE}" ]]; then
    echo "--baseline-file is required" >&2
    exit 1
fi

if [[ -n "${LOG_FILE}" && -n "${DOCKER_CONTAINER}" ]]; then
    echo "pass only one of --log-file or --docker-container" >&2
    exit 1
fi

if [[ -z "${LOG_FILE}" && -z "${DOCKER_CONTAINER}" ]]; then
    echo "one of --log-file or --docker-container is required" >&2
    exit 1
fi

if [[ ! -f "${BASELINE_FILE}" ]]; then
    echo "missing baseline file: ${BASELINE_FILE}" >&2
    exit 1
fi

BASELINE_OFFSET="$(tr -d '[:space:]' < "${BASELINE_FILE}")"
if [[ ! "${BASELINE_OFFSET}" =~ ^[0-9]+$ ]]; then
    echo "invalid baseline offset in ${BASELINE_FILE}: ${BASELINE_OFFSET}" >&2
    exit 1
fi

TMP_LOG="$(mktemp)"
TMP_FRAGMENT="$(mktemp)"
trap 'rm -f "${TMP_LOG}" "${TMP_FRAGMENT}"' EXIT

capture_log_since_baseline() {
    if [[ -n "${DOCKER_CONTAINER}" ]]; then
        docker logs "${DOCKER_CONTAINER}" >"${TMP_LOG}" 2>&1
    else
        cp "${LOG_FILE}" "${TMP_LOG}"
    fi

    tail -c "+$((BASELINE_OFFSET + 1))" "${TMP_LOG}"
}

has_pattern() {
    local pattern="$1"
    capture_log_since_baseline >"${TMP_FRAGMENT}"
    grep -Eq "${pattern}" "${TMP_FRAGMENT}"
}

START_SECONDS="$(date +%s)"

while true; do
    submitted=0
    chunk=0

    if has_pattern "submitting chunk proof tasks for sealed batch|submitting chunk \\+ acct proof tasks"; then
        submitted=1
    fi
    if has_pattern "marking chunk as proof-ready"; then
        chunk=1
    fi

    echo "EEST proof signals: submitted=${submitted} chunk=${chunk}"

    if ((submitted && chunk)); then
        if has_pattern "retries exhausted|task died mid-Proving and retries exhausted"; then
            echo "observed permanent prover failure after EEST baseline" >&2
            exit 1
        fi
        echo "EEST-generated blocks reached the EE chunk proof pipeline"
        exit 0
    fi

    NOW_SECONDS="$(date +%s)"
    ELAPSED_SECONDS=$((NOW_SECONDS - START_SECONDS))
    if ((ELAPSED_SECONDS >= TIMEOUT_SECONDS)); then
        echo "timed out waiting for EEST proof signals after ${TIMEOUT_SECONDS}s" >&2
        echo "last log tail after baseline:" >&2
        capture_log_since_baseline | tail -200 >&2
        exit 1
    fi

    sleep "${POLL_SECONDS}"
done
