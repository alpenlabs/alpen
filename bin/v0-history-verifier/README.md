# V0 history importer

Run this tool from the importer branch based on `v0.3.5` on a disposable, offline copy of a node datadir. Sled needs exclusive writable access for locking and recovery. Pass the independently recorded genesis ID and promotion terminal for that network.

```sh
cargo run --locked -p strata-v0-history-verifier -- \
  --datadir "$copy" --genesis-id "$genesis_id" \
  --terminal-slot "$slot" --terminal-epoch "$epoch" --terminal-id "$terminal_id" \
  --source-snapshot "$snapshot_id" --target-datadir "$promoted_copy"
```

Omit both `--source-snapshot` and `--target-datadir` to verify the source alone. For an import, stop the target node and pass its existing, promoted datadir. Its stored history base must match the supplied terminal slot, epoch and block ID.

The importer walks the parent chain from the terminal to genesis, flushes the destination, then walks the locked source again and compares every stored value. It checks row counts, computes the archive checksum, and writes and flushes `V0OLArchiveMeta[complete]` last. A rerun with the same inputs can finish an interrupted import. A completed import is protected against replacement.

## Archive format 1

The current node owns the [sled schema and completion record](https://github.com/alpenlabs/alpen/blob/e33db2c6376b208b2d745e546c524bb75ba5329f/crates/db/store-sled/src/ol/schemas/v0_history.rs). This release branch includes the same file under `crates/db/store-sled/src/ol/schemas/v0_history.rs` so the offline binary can build against the V0 codecs.

| Tree | Key | Value |
| --- | --- | --- |
| `V0OLHeaderBySlot` | Eight-byte big-endian slot | Unsigned header SSZ |
| `V0OLBlockById` | Original 32-byte block ID | Original signed V0 block bytes |
| `V0OLWriteBatchById` | Block ID | Original V0 write batch bytes |
| `V0OLTransactionsById` | Block ID | V0 RPC transaction array as UTF-8 JSON, in body order |
| `V0OLAccountChangesById` | Block ID | V0 RPC account changes as UTF-8 JSON |
| `V0OLArchiveMeta` | ASCII `complete` | JSON with format version, source, decoder revision, genesis and terminal, counts, and content SHA-256 |

Genesis has a block, header and transaction row. Batch and account-change rows start at slot 1, including blocks with empty batches. The five explorer RPCs use these trees for historical headers, raw blocks by range and ID, transactions, and account changes.

The checksum covers the five data trees in table order. Hash the domain string `strata/v0-sled-archive/1`, then each tree name, its eight-byte big-endian row count, and each key and value in sled key order. Prefix every string, key and value with its eight-byte big-endian byte length.
