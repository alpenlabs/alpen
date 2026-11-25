# V0 history importer

This offline tool uses the v0.3.5 codecs. The current node owns the [sled tree layout and completion record](https://github.com/alpenlabs/alpen/pull/2478). Only the five data trees and the metadata tree belong to the importer. Both databases require exclusive writable access for sled locking and recovery.

## Prepare

Check out the approved importer tag and build on a compatible host. Record the full commit and binary checksum. Restore the V0 source snapshot to a disposable volume and mount the datadir containing `sled/strata-client/db`. Obtain the expected genesis and promotion terminal independently from the network configuration and cutover record.

```bash
git checkout --detach "$IMPORTER_TAG"
revision=$(git rev-parse HEAD)
cargo build --release --locked -p strata-v0-history-importer
tool=target/release/strata-v0-history-importer
sha256sum "$tool"
terminal=(--terminal-slot "$slot" --terminal-epoch "$epoch" --terminal-id "$terminal_id")
source_args=(--datadir "$source_copy" --genesis-id "$genesis_id" "${terminal[@]}")
"$tool" verify "${source_args[@]}"
```

## Rehearse, then import

For rehearsal, restore a copy of the promoted node’s database. For a production window, pause signing, stop the sequencer and snapshot its active volume. Set `target` to the stopped promoted datadir that will be served. If cutting over to a clone, create it from that window’s stopped snapshot and keep the original claim.

```bash
set -o pipefail
"$tool" import "${source_args[@]}" --target-datadir "$target" \
  --source-snapshot "$source_snapshot" --importer-revision "$revision" | tee import.json
jq .complete import.json
```

Import checks the target’s stored history base before opening any archive trees. It clears unpublished data, walks the source, flushes the target and compares every value on a second walk. Genesis has no write batch. The last flush publishes the completion record. Retry a partial import with the same arguments. A completed import requires explicit removal before replacement.

Before production, start the approved reader image on the imported rehearsal copy, isolated from signing and L1 submission. Check its verification log and the five RPCs (headers and raw blocks by range, raw block by ID, transactions and account changes). Check genesis and a range crossing into live history. Repeat these checks on the cutover volume before resuming signing. Retain the importer tag, commit, binary hash, source and target snapshot IDs, arguments, import report and completion record for each run.

## Remove an archive

Stop all users of the target sled database. Use the latest target volume and its independently recorded terminal. Removal checks the same history base, deletes and flushes the completion record first, drops only the six named trees, and flushes again. Retry after interruption. To replace the archive, run `import` again and repeat readback. Never rewind signed history or pending L1 submissions to undo an archive import.

```bash
"$tool" remove --target-datadir "$target" "${terminal[@]}"
```

## Fixture checks

```bash
cargo test --locked -p strata-v0-history-importer
bin/v0-history-importer/interop/run.sh "$reader_checkout" "$reader_commit"
```

The default tests restore the logical contents of a reader-created promoted database (`testdata/promoted.json`, including empty trees and two canonical live successors). A subprocess exits during the write walk after a persisted block. The test checks the digest of every other tree, including names and empty trees, after the interruption, retry, removal and reimport. The interop script creates a disposable checkout of the specified reader commit. It uses the node’s promotion API, imports the generated chain with the CLI and compares all five registered RPCs with the v0.3.5 source. The script prints its local workspace path for inspection.
