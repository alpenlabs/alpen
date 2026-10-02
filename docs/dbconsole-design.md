# A smaller, extensible database console

See [`dbconsole-implementation-plan.md`](dbconsole-implementation-plan.md) for the proposed delivery sequence.

## 1. The problem

`strata-dbtool` has 30 top-level commands, 13 command modules, and parallel output types. Even a simple read or record update needs its own arguments, dispatch, storage call, presentation type, rendering, tests, and documentation. The production codecs and domain setters are sound; the repeated command plumbing is the problem.

The prover-task read and reset paths show the repetition across `bin/strata-dbtool/src/cli.rs`, `main.rs`, and `cmd/prover_task.rs`, before reaching the production store in `crates/db/store-sled/src/prover/`. By contrast, `revert-ol-state` coordinates several stores and enforces finalization, checkpoint, and MMR invariants. The replacement should generalize the first kind of work without turning the second into a script.

## 2. Coverage

The 30 current operations fall into three groups. The classification measures whether the operator task fits the mechanism, not whether every legacy JSON object stays byte-for-byte identical.

| Mechanism | Count | Share | Representative work |
|---|---:|---:|---|
| Generic table query or typed modifier | 9 | 30.0% | Prover-task reads and updates, summaries over one table |
| Registered native read view | 14 | 46.7% | OL block, checkpoint, and sync information assembled from context |
| Native mutation or recovery recipe | 7 | 23.3% | State reversion, backfills, and coordinated deletion |

Generic operations plus native views cover 23 of 30 commands, or 76.7%. Functional tests contain 66 literal command references, 49 of them to operations classified as views. These are static references, not production telemetry, but they make the missing abstraction clear: we need reusable native views more than we need a more powerful language. Recipes give the remaining seven operations an explicit home.

## 3. The model

Database values remain concrete Rust values. An **opaque record handle** refers to a decoded Rust record held by the console. Pipelines can invoke approved getters or modifiers on the handle, but cannot enumerate, reconstruct, or replace the complete object.

```text
stored bytes -> production decoder -> concrete Rust value
                                      | opaque handle
                                      v
                         approved getters and modifiers
                                      |
                         streamed scalar rows / staged writes
```

Internally, a handle contains a table identifier, typed key, and owned type-erased value such as `Box<dyn Any + Send>`. The table registration performs checked downcasts, so a handle from one table cannot be written as another table's value.

There are four registrations: `ConsoleValue` exposes approved fields and modifiers, `ConsoleTable` connects a value to storage, `ConsoleView` implements trusted reads across tables, and `ConsoleRecipe` owns complex repairs.

The query syntax has field references, literals, parentheses, and fixed arithmetic, comparison, and boolean operators. It has no user-defined control flow, functions, types, modules, or host calls.

A scan is the loop. `scan` walks forward, `scan_rev` walks backward, and each following operation is applied to every yielded handle. `filter` is the conditional, `select` is the constrained map, and a terminal aggregation is the reduction. There is no need for a general `for` block or callback:

```text
scan ProverTask
| filter status == "Pending" && metadata_len > 0
| select key, status, metadata_len + 4 as framed_len
| take 20
```

The language provides fixed aggregators such as `count`, `sum`, `min`, `max`, `any`, `all`, and possibly `group_count`. User-supplied `reduce`, `zip`, and arbitrary `map` stay out.

Scalar evaluation is strict: arithmetic is checked, division by zero fails, and incompatible types do not coerce. `null` supports equality only. Getter metadata lets the console check an expression before starting the scan.

## 4. Reads and pipelines

A getter borrows the Rust value and returns a small scalar. A name like `batch.last_block` is a registered projection, not a path through a reflected object tree. Scans decode one owned value at a time with the production `ValueCodec`. Ownership allows later staging; pull-based traversal avoids collecting the table as `list_all_tasks` currently does.

`take`, `first`, and `last` stop traversal early; `last` can request reverse traversal rather than consuming a complete forward scan. Registrations must say whether encoded key order has useful domain meaning. Otherwise, `scan` and `scan_rev` are documented as storage-key order.

Interactive output has a conservative default cap. `take` or an output option raises it. The executor checks for cancellation between rows, so Ctrl-C stops a scan without leaving state behind.

Values and aggregates use the existing porcelain and JSON conventions. Streams also support JSON Lines. The same evaluator works noninteractively:

```bash
strata-dbconsole --datadir data eval \
  'scan ProverTask | filter status == "Pending" | select key, status | take 20' \
  --format jsonl
```

Compatibility wrappers keep existing subcommand output stable during migration.

## 5. Safe writes

The console is read-only by default. Write mode requires the node to be stopped and an exclusive Sled open. A modifier parses scalar arguments and calls domain methods on the real type; there is no arbitrary field assignment or JSON replacement. For example, `TaskRecordData::set_status` also updates `updated_at_secs`, while direct assignment would break that invariant.

Point and bulk changes use the same pipeline:

```text
get ProverTask deadbeef | modify reset | stage

scan ProverTask
| filter status == "Pending" || status == "Proving"
| modify abandon "operator cancelled"
| stage all
```

Staging a stream requires a bound such as `take 20` or an explicit `all`. Cancellation or one modifier failure discards the new batch.

The session holds writes as `Vec<Box<dyn StagedWrite>>`, retaining each typed key, value, registration, and pre-edit fingerprint. `staged` previews the batch, `abort` drops it, and `commit` rechecks fingerprints before using production storage APIs. Once apply begins, cancellation waits rather than interrupting a commit mid-write.

The current high-level database traits do not expose one arbitrary transaction spanning any set of Sled trees. A generic commit should therefore contain writes for only one table registration, which may supply its own atomic multi-row apply function. Cross-table or ordered mutations remain recipes until a production transaction coordinator exists. This avoids promising atomicity that the storage APIs do not provide.

## 6. Registration and use

For repo-owned values, derive on the value itself. The macro can see private fields and generate getter dispatch without reconstructing the value. Foreign values, or crates avoiding the metadata dependency, use a local adapter.

The table still needs a thin registration because a value does not know which schema stores it, how its key is written, or which production API should persist it. The implemented read-side shape is:

```rust
#[cfg_attr(feature = "db-console", derive(strata_db_console::ConsoleValue))]
#[cfg_attr(
    feature = "db-console",
    console(
        modifier(name = "reset", with = TaskRecordData::console_reset)
    )
)]
pub struct TaskRecordData {
    #[cfg_attr(
        feature = "db-console",
        console(get(scalar = "string", with = TaskRecordData::console_status))
    )]
    status: TaskStatus,

    #[cfg_attr(feature = "db-console", console(get))]
    retry_after_secs: Option<u64>,

    // Other stored fields are not exposed unless they are registered above.
}

impl TaskRecordData {
    fn console_status(&self) -> String {
        // Explicit domain projection.
        status_name(&self.status).to_owned()
    }

    fn console_reset(&mut self) -> strata_db_console::ConsoleResult<()> {
        self.set_status(TaskStatus::Pending);
        self.set_retry_after_secs(None);
        Ok(())
    }
}

#[derive(strata_db_console::ConsoleTable)]
#[console(
    name = "ProverTask",
    alias = "tasks",
    schema = ProverTaskTree,
    value = TaskRecordData,
    adapter = SledConsoleTable,
    parse_key = parse_byte_key,
    render_key = render_byte_key,
    map_value = identity
)]
struct ProverTaskConsoleTable(SledTree<ProverTaskTree>);

let mut registry = ConsoleRegistry::new();
registry.register(Arc::new(ProverTaskConsoleTable(tree.clone())))?;
```

For `#[console(get)]`, the field name becomes the getter name, `Option<T>` becomes nullable, and the scalar type is inferred for `bool`, `i64`, `u64`, `String`, and `Vec<u8>`. A projection only names what differs. `ConsoleValue` generates metadata, scalar conversion, getter dispatch, modifier argument checks, and modifier dispatch. `ConsoleTable` generates the table trait implementation but delegates reads and scans to the named adapter; key parsing, key rendering, decoded-value mapping, and storage semantics stay explicit. The macros cannot infer domain semantics or cross-table invariants, and they do not perform field assignment or persistence. Versioned registrations select the production decoder and fail closed on unsupported versions.

The resulting interaction uses only registered capabilities:

```text
$ strata-dbconsole --datadir data

db> schema ProverTask
key: hex bytes
getters: status, retry_after_secs, metadata_len, updated_at_secs
modifiers: reset, abandon(reason)

db> get ProverTask deadbeef
status: Proving; retry_after_secs: 1700000000; metadata_len: 32

db> scan_rev ProverTask
  | filter status == "Pending" && metadata_len > 0
  | select key, status, metadata_len
  | take 20

db> scan ProverTask | group_count status
Pending: 12
Proving: 2
Completed: 103

db> get ProverTask deadbeef | modify reset | stage
staged 1 change

db> staged
ProverTask deadbeef: status Proving -> Pending; retry_after_secs 1700000000 -> null

db> commit
committed 1 change
```

## 7. Views, recipes, and Rhai

A native view prevents cross-table reads from leaking complexity into the expression language. It is a registered Rust source with typed arguments and a derived output value:

```rust
#[derive(ConsoleValue)]
struct SyncInfo {
    #[console(get)]
    ol_tip_height: u64,
    #[console(get)]
    current_epoch: u32,
    #[console(get)]
    finalized_epoch: EpochCommitment,
}

#[console_view(name = "SyncInfo")]
fn sync_info(db: &SledBackend, l1_reorg_safe_depth: u32) -> Result<SyncInfo> {
    // Reuse the authoritative cross-table computation.
}
```

An operator can then write `view SyncInfo l1_reorg_safe_depth=6 | select ol_tip_height, finalized_epoch`. The view owns the join and the finality rule; the common console owns arguments, composition, output, and automation.

Native recipes own branching, nested scans, coordinated mutations, and cross-table invariants. Existing `revert-ol-state`, `delete-ol-block`, and `backfill-terminal-headers` belong here. They can use the console's preview and explicit commit vocabulary, but their plans and validation remain reviewed Rust.

Rhai is not inherently unsafe, but it brings a larger contract: host APIs, resource limits, failure semantics, and arbitrary scripts to support. The current workflows do not need it. If variables, user-defined functions, or nested control flow become real requirements, use the Rhai direction demonstrated by `origin/ee-storage-dbconsole` in `../alpen-ee`; do not grow this language one feature at a time.

## 8. MVP and migration

Start with a separate `strata-dbconsole` binary beside `strata-dbtool`. A useful first release should include:

- value- and table-level proc macros;
- `get`, `scan`, `scan_rev`, scalar expressions, `filter`, `select`, `take`, `first`, `last`, and fixed aggregators;
- porcelain, JSON, and JSON Lines output in interactive and `eval` modes;
- one writable table with `modify`, bounded or explicit-all staging, `staged`, `commit`, and `abort`;
- two representative native views, preferably `OlBlock` and `SyncInfo`;
- adapters that invoke existing high-risk repair commands as native recipes.

Migrate simple `get-*` and summary operations by registering their values and tables. Migrate composite reads to views while retaining compatibility wrappers for existing scripts and functional tests. Move safe record changes to typed modifiers only after their previews and validation match current behavior. Keep recovery logic native throughout the migration.

Do not reuse the `strata-dbtool` binary name initially. Coexistence makes output and behavior changes explicit and gives automation a deprecation window. Once the console covers the exercised workflows and compatibility wrappers are stable, `strata-dbtool` can become an alias or be retired.

Three implementation questions should be settled before the public APIs are fixed: whether console runtime traits live in `strata-db-store-sled` or a narrow companion crate, which legacy porcelain and JSON fields require byte-for-byte stability, and whether generic deletion belongs in the MVP or follows after typed modification.

**Recommendation:** build the opaque-handle console with derived value metadata, thin table registrations, a fixed scalar expression language, native read views, and native recovery recipes. This design directly generalizes the common record workflows, accommodates all 30 current operations without pretending that cross-table recovery is generic, and provides a clear point at which an embedded language would become the more honest choice.
