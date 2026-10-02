# Database console implementation plan

This plan implements the design in [`dbconsole-design.md`](dbconsole-design.md). The REPL comes last. We first prove that real database values can be exposed cleanly, then add reads, safe point writes, functional pipelines, and finally a text frontend.

There is one small change from the proposed order: hand-write the first table mappings before fixing the proc-macro API. The Sled schema types and tree fields are private to `strata-db-store-sled`, so the correct registration boundary is not obvious from the value types alone. Once two real mappings work, the macro can generate the repeated parts of a known-good shape.

## 1. Delivery order

| Phase | Outcome |
|---|---|
| 1. Mapping spike | A registry can expose simple and complex Sled values without a parser or CLI. |
| 2. Proc macros | The successful manual mappings are reduced to value and table declarations. |
| 3. Read path | Programmatic `schema`, `get`, `scan`, and native views work with stable output rows. |
| 4. Safe point writes | One record can be modified, staged, previewed, aborted, and committed. |
| 5. Functional primitives | Scans gain expressions, filtering, projection, aggregation, and bounded bulk writes. |
| 6. Frontend and migration | `eval` is added first; the interactive REPL is the final wrapper. |

Each phase should compile and be useful on its own. Do not start by porting all current dbtool commands.

## 2. Mapping spike

Create a small storage-independent crate, tentatively `strata-db-console`, for `ConsoleScalar`, `ConsoleValue`, opaque handles, descriptors, and the registry. It must not depend on Sled, `strata-db-types`, or domain crates. This keeps it low enough in the dependency graph for database values to implement or derive its traits.

Put Sled table registrations next to the concrete database implementations in `strata-db-store-sled`. Those modules can access private trees and production codecs without making schema types public or adding console-only scans to the broad database traits. A registration owns or clones the concrete database handle and exposes typed `get`, forward scan, and reverse scan operations through the storage-independent interface.

Compile console metadata and registrations normally rather than placing feature gates on derives, fields, helpers, and imports. The generated code is passive until a caller explicitly builds the registry. Only derived values expose fields, and only registered tables are reachable.

Prove the boundary with these mappings:

- `OLBlockStatusSchema: OLBlockId => BlockStatus` from `crates/db/store-sled/src/ol/schemas.rs` as the simple case. It exercises a typed key and small enum value.
- `ProverTaskTree: Vec<u8> => TaskRecordData` from the prover schema and `crates/prover-core/src/task.rs` as the complex case. It exercises private fields, byte keys, projections, and invariant-preserving modifiers.
- `OLBlockSchema: OLBlockId => OLBlockV1` as the adapter case. Expose a few header projections locally rather than adding console metadata to the consensus type.

Start with handwritten `ConsoleValue` and table registrations. The spike is done when a unit test can build the registry, fetch one value from each mapping, invoke its approved getters, and lazily consume the first few scan results. There is still no command parser, evaluator, or binary.

## 3. Proc macros

Add a separate `strata-db-console-macros` proc-macro crate and make the runtime crate re-export its derives. Keep the generated API small:

- `ConsoleValue` generates getter metadata, scalar projection dispatch, and calls to explicitly declared field setters.
- `ConsoleTable` generates key parsing, table metadata, handle construction, broader modifier dispatch, and calls into explicit storage hooks.
- Projection, setter, and modifier functions remain normal Rust. The macro must never invent field assignment or storage semantics.

Apply the value derive directly and unconditionally to repo-owned values selected for the console. Keep local adapters for foreign types and consensus types that should not gain console annotations. Keep one explicit registry assembly function; compiling a derive does not register a value or activate the console.

Use four field forms and define them in the macro documentation: direct `get`, projected `get(via = ...)`, conventional `set`, and custom `set(via = ...)`. A bare `set` calls `set_<field>` and fails to compile if the method is absent or has the wrong type. A field without `set` is read-only. Use `via` only to mean “call this Rust function.”

Use the handwritten mappings as the acceptance test: replace their boilerplate with derives without changing their observed metadata or getter results. Do not add linker registration or automatic crate discovery. Keep one explicit registry assembly function so the supported surface is easy to audit.

## 4. Read path

Build the executor as a Rust API before designing syntax. It should accept a small typed plan assembled directly in code and return a single value, stream of rows, or aggregate. Implement only `schema`, `get`, `scan`, `scan_rev`, and a mandatory bound for interactive-style scans. `take` belongs here as a safety mechanism even though the richer functional layer comes later.

Use owned decoded values in handles and pull records from the typed Sled iterator one at a time. Define storage order explicitly for every table. Add the common row renderer here: porcelain for humans, JSON for single results, and JSON Lines for streams.

Keep reverse scans on Sled's native double-ended iterator. `typed_sled::SledTreeIter` delegates `next_back` to `sled::Iter`, so `scan_rev` must remain lazy and must not collect a table merely to reverse it.

Then implement three vertical read slices:

1. Direct table reads for prover tasks, including point lookup and a bounded scan.
2. An `OlBlock` native view that combines block data with status.
3. A `SyncInfo` native view that reuses the existing cross-table and finality logic.

The views matter more than broad table coverage: the command audit showed that composite reads dominate current test usage. Do not port all 23 read operations yet. These three slices are enough to validate simple records, nested records, and cross-table context.

## 5. Safe point writes

Add writes only after the read handles and table ownership model have settled. The first writable value should be `TaskRecordData`. Expose `retry_after_secs` as a field setter that calls `TaskRecordData::set_retry_after_secs`; keep fields without setters read-only. Register `reset` and `abandon` as broader table modifiers, and implement them through existing domain methods. Start with point writes only:

```text
get ProverTask <key> | set retry_after_secs null | stage
staged
commit
```

`get ProverTask <key> | modify reset | stage` follows the same path for a broader operation.

Setters and modifiers share the same staging path. The session stores a type-erased staged write containing the typed key, original and replacement values, description, and stable preview. Point commits use typed Sled compare-and-swap with the original encoded value, so stale session state cannot overwrite a newer record.

A generic commit contains writes for one table registration. Cross-table writes remain recipes. Once storage apply starts, cancellation waits for it to return. Do not implement generic delete, insert, or structural replacement in this phase.

## 6. Functional primitives

With reads and point writes working, add an expression AST and evaluator without textual syntax. Implement the primitives in this order:

1. comparisons and boolean operators;
2. `filter`, `select`, and `take`;
3. checked arithmetic;
4. `count`, `first`, `last`, `min`, `max`, `sum`, `any`, and `all`.

Keep scalar conversions strict and validate the expression against getter metadata before starting a scan. Do not add general `map`, user reducers, variables, callbacks, `zip`, or grouping without a concrete operator workflow that cannot be expressed with the fixed primitives.

After filtering is stable, allow bounded bulk modification. `scan | filter | modify | stage` must first build a temporary batch; cancellation or one failed modifier discards it. The initial API always requires a scan bound and may apply a smaller `take N` after filtering. The commit rechecks every original value and applies the same-table replacements in one Sled transaction. This is also the point to replace the prover-task summary and bulk-abandon loops with pipelines and compare their output with the current commands.

## 7. Frontend and migration

Add one parser only after the executor API is stable. The parser produces the same typed plan used by the programmatic tests. First expose it through:

```text
strata-dbconsole --datadir <path> eval '<pipeline>' --format jsonl
```

Then add the REPL as a thin loop around that parser and executor, with multiline pipelines, `schema`, `staged`, `commit`, `abort`, help, and Ctrl-C cancellation. Do not put database behavior in the REPL layer.

Ship `strata-dbconsole` beside `strata-dbtool`. Migrate more direct reads and native views only after the initial slices are useful. Keep recovery commands in dbtool or register them as native recipes; do not translate `revert-ol-state`, `delete-ol-block`, or backfills into the expression language. Compatibility wrappers can preserve existing command output while functional tests move gradually.

## 8. Test and commit discipline

The goal is a small number of tests at the abstraction boundaries, not a test matrix for generated plumbing. Aim for one to three focused tests per phase:

- one derive/metadata test and one rejected macro declaration;
- one registry test covering the simple and complex mappings;
- one lazy read test proving early termination;
- one point-write test covering a domain setter, a broader modifier, preview, abort, and commit;
- a table-driven scalar evaluator test; and
- one bulk-staging failure test proving all-or-nothing behavior.

Reuse the existing temporary-Sled fixtures. Avoid snapshots, exhaustive operator-by-type combinations, and duplicate tests for code emitted by the same macro. The frontend eventually needs only a noninteractive smoke test and a REPL parsing/cancellation test; existing dbtool functional tests remain the compatibility evidence during migration.

Keep commits aligned with the phases, and run focused `cargo check` or tests for the touched crates after each one. Run workspace formatting and linting once a phase is ready for review rather than expanding every implementation step into a large test project.
