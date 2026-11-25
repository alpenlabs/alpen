#!/usr/bin/env bash
# Run from the v0.3.5 tooling checkout with the reader checkout and revision as arguments.
set -euo pipefail
: "${1:?reader checkout}" "${2:?reader commit}"
root=$(git rev-parse --show-toplevel)
tools="$root/bin/v0-history-importer/interop"
work=$(mktemp -d)
export STRATA_V0_FIXTURE_OUT="$work/fixture"
echo "Cross-release fixture and disposable reader checkout: $work"
git clone --quiet --local --no-checkout "$1" "$work/reader"
git -C "$work/reader" checkout --quiet --detach "$2"
rpc="$work/reader/bin/strata/src/rpc"
mkdir -p "$rpc/assembly_tests" "$rpc/node_tests"
export STRATA_PROMOTED_FIXTURE="$root/bin/v0-history-importer/testdata/promoted.json"
cp "$tools/read.rs" "$rpc/assembly_tests/roundtrip.rs"
cp "$tools/prepare.rs" "$rpc/node_tests/roundtrip.rs"
for module in assembly_tests node_tests; do
    printf '\nmod roundtrip;\n' >> "$rpc/$module.rs"
done
cargo test --locked -p strata-v0-history-importer --test offline write_reader_fixture -- --ignored --exact
reader_test() {
    (cd "$work/reader" && cargo test --locked -p strata --bin strata "$1" -- --ignored --exact)
}
reader_test rpc::node_tests::roundtrip::prepare_promoted_destination
terminal=(--terminal-id "$(jq -er '.[2].header.blkid' "$STRATA_V0_FIXTURE_OUT/responses.json")" --terminal-epoch 1 --terminal-slot 2)
target=(--target-datadir "$STRATA_V0_FIXTURE_OUT/promoted" "${terminal[@]}")
tool=(cargo run --locked -p strata-v0-history-importer --)
"${tool[@]}" import "${target[@]}" --datadir "$STRATA_V0_FIXTURE_OUT/source" \
    --genesis-id "$(jq -er '.[0].header.blkid' "$STRATA_V0_FIXTURE_OUT/responses.json")" \
    --source-snapshot local-fixture --importer-revision "$(git rev-parse HEAD)"
reader_test rpc::assembly_tests::roundtrip::explorer_reads_imported_database
"${tool[@]}" remove "${target[@]}"
