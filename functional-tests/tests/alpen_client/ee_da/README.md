# EE DA Functional Test Boundaries

This suite checks the EE DA publication path: chunked-envelope transport, blob
reassembly from canonical L1 blocks, producer-local byte parity, and local
state reconstruction from DA diffs.

It does not replace sync or lifecycle coverage. Reorg handling, fullnode catchup,
and long-running DA lifecycle behavior belong in their existing sync, restart,
and transport-focused tests.
