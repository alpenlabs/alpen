# Bitcoin publication recovery

At runtime, the broadcaster, payload watcher, and bundler remain critical services.
Recoverable errors reaching their service boundaries defer work with exponential backoff,
jittered from half to all of a delay that doubles from one second to a 30-second
ceiling. Normal polling intervals still apply. The broadcaster's RPC adapters also
return `RetryLater` outcomes, which retry at the normal polling cadence.

Database retries recognize `Busy` and I/O kinds `Interrupted`, `WouldBlock`, and
`TimedOut`, including wrapped retry-exhaustion errors. Other I/O errors, codec
failures, and failed database workers remain non-retryable. Sled can retain
an I/O failure until reopened; retry classification does not guarantee recovery.

The bundler retains unfinished intent indexes. The broadcaster reloads persisted
tracking after a failed pass. The watcher retains signed attempts across failed
writes and resumes storing the same transaction pair. These caches are in memory;
this change does not add recovery guarantees across process restarts.

Run `cd functional-tests && ./run_tests.sh -t test_bitcoin_rpc_outage` to verify
publication recovery after a Bitcoin Core restart. Unit tests inject DB failures
at the service boundary. Other critical tasks, including ASM, have separate error
policies; this test keeps ASM caught up before stopping Bitcoin Core.
Signing-key rotation follows the existing policy and is not exercised by this test.
