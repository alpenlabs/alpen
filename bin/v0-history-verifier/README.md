# V0 history exporter

Run this tool from the `v0.3.5` checkout on a disposable, offline copy of a node datadir. Sled needs exclusive writable access for locking and recovery. Pass the independently recorded genesis ID and promotion terminal for that network.

```sh
cargo run --locked -p strata-v0-history-verifier -- \
  --datadir "$copy" --genesis-id "$genesis_id" \
  --terminal-slot "$slot" --terminal-epoch "$epoch" --terminal-id "$terminal_id" \
  --source-snapshot "$snapshot_id" --archive-dir "$archive"
```

Omit both `--source-snapshot` and `--archive-dir` for verification alone. For an export, the destination must be new and its parent directory must exist. Both modes walk the terminal's parent chain through genesis and validate every block and batch. The exporter publishes the directory after the complete walk and SQLite commit succeed.

## Archive format 1

`manifest.json` records the source snapshot supplied by the operator, the V0 decoder revision, genesis and terminal IDs, the inclusive slot range, observed counts and the SHA-256 of the complete `history.sqlite3` file. Before serving the archive, the consumer must check the format version, checksum and expected network endpoints.

Open SQLite read-only. The `blocks` table contains one row per verified slot, keyed by integer `slot` and uniquely indexed by the 32-byte binary `block_id`. For both ranges use inclusive bounds and `ORDER BY slot`. The archive holds the following data for the RPC layer.

| Explorer RPC | Stored data |
| --- | --- |
| `strata_getHeadersInRange` | `header` contains V0 `RpcBlockHeaderEntry` JSON, including the parent ID and all header roots. |
| `strata_getRawBlocksRange` | `slot`, `epoch`, `block_id` and `raw_block` supply each range entry. The blob contains the original signed SSZ bytes from sled. |
| `strata_getRawBlockById` | Look up the same `raw_block` by binary `block_id`. Encode it as RPC hex at the wire boundary. |
| `strata_getBlockTransactions` | `transactions` contains the JSON array of V0 `RpcOLTxDetail` values in body order, including the V0 transaction IDs. |
| `strata_getBlockAccountChanges` | `account_changes` contains V0 `RpcBlockAccountChanges` JSON with each changed account's post-state. A present batch can yield an empty `changes` list. |

At slot 0, SQL `NULL` in `account_changes` represents the absent genesis write batch. The RPC reader must preserve the V0 error code `-32602` and message `No OL write batch found for block {block_commitment}` for that slot. Every later slot requires a batch at its exact block commitment.
