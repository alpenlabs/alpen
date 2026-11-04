# Bitcoin publication error handling

The broadcaster, payload watcher, and bundler are critical services. Errors that
escape their service boundaries shut down the node, but transient Bitcoin RPC
errors already have narrower handling:

- The broadcaster converts retryable lookup and publication failures into
  `RetryLater` outcomes and tries again on a later poll.
- The watcher defers envelope construction and signing after retryable RPC
  failures. Its replacement pass also retries on later polls.
- The bundler performs database operations, with no Bitcoin RPC calls.

`src/rpc_error.rs` classifies connection failures, timeouts, exhausted client
retries, HTTP 5xx responses, and Bitcoin Core warmup as retryable. Other failures
retain their existing handling. Unhandled database errors propagate: sled documents
its errors as generally fail-stop and can retain an I/O failure until reopened, so
an I/O error kind alone does not establish that retrying is safe or useful.

Run `cd functional-tests && ./run_tests.sh -t test_bitcoin_rpc_outage` to check
process liveness during a 20-second Bitcoin Core outage and new checkpoint
publication after recovery. This regression also passes against the pre-change
node at `4c972814a`. It keeps ASM caught up before the outage; it does not establish
outage recovery for other critical services or simulate database faults.
