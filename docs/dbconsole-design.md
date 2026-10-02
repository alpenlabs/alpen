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

Database values remain concrete Rust values. An **opaque record handle** refers to a decoded Rust record held by the console. Pipelines can invoke approved getters, field setters, or table modifiers on the handle, but cannot enumerate, reconstruct, or replace the complete object.

```text
stored bytes -> production decoder -> concrete Rust value
                                      | opaque handle
                                      v
                    approved getters, setters, and modifiers
                                      |
                         streamed scalar rows / staged writes
```

Internally, a handle contains a table identifier, typed key, and owned `Box<dyn ConsoleValue>`. The table registration performs checked downcasts, so a handle from one table cannot be written as another table's value.

There are four registrations: `ConsoleValue` exposes approved fields and safe field setters, `ConsoleTable` connects a value to storage and owns broader modifiers, `ConsoleView` implements trusted reads across tables, and `ConsoleRecipe` owns complex repairs.

The query syntax has field references, literals, parentheses, and fixed arithmetic, comparison, and boolean operators. It has no user-defined control flow, functions, types, modules, or host calls.

A scan is the loop. `scan` walks forward, `scan_rev` walks backward, and each following operation is applied to every yielded handle. `filter` is the conditional, `select` is the constrained map, and a terminal aggregation is the reduction. There is no need for a general `for` block or callback:

```text
scan ProverTask
| filter status == "pending" && metadata_len != null && metadata_len > 0
| select key, status, metadata_len + 4 as framed_len
| take 20
```

The language provides fixed aggregators such as `count`, `sum`, `min`, `max`, `any`, and `all`. User-supplied `reduce`, `zip`, arbitrary `map`, and grouping stay out until a concrete operator workflow justifies them.

Scalar evaluation is strict: arithmetic is checked, division by zero fails, and incompatible types do not coerce. `null` supports equality only. Getter metadata lets the console check an expression before starting the scan.

## 4. Reads and pipelines

A getter borrows the Rust value and returns a small scalar. A name like `batch.last_block` is a registered projection, not a path through a reflected object tree. Scans decode one owned value at a time with the production `ValueCodec`. Ownership allows later staging; pull-based traversal avoids collecting the table as `list_all_tasks` currently does.

`take`, `first`, and `last` stop traversal early; `last` can request reverse traversal rather than consuming a complete forward scan. Registrations must say whether encoded key order has useful domain meaning. Otherwise, `scan` and `scan_rev` are documented as storage-key order.

Interactive output has a conservative default cap. `take` or an output option raises it. The executor checks for cancellation between rows, so Ctrl-C stops a scan without leaving state behind.

Values and aggregates use the existing porcelain and JSON conventions. Streams also support JSON Lines. The same evaluator works noninteractively:

```bash
strata-dbconsole --datadir data eval \
  'scan ProverTask | filter status == "pending" | select key, status | take 20' \
  --format jsonl
```

Compatibility wrappers keep existing subcommand output stable during migration.

## 5. Safe writes

The console is read-only by default. Write mode requires the node to be stopped and an exclusive Sled open. There is no arbitrary field assignment or JSON replacement. A field setter calls an existing domain setter, and a broader modifier calls domain methods on the real type. For example, `TaskRecordData::set_status` also updates `updated_at_secs`, while direct assignment would break that invariant.

A field without a registered setter remains read-only. `#[console(get, set)]` never generates `self.field = value`; it generates a call to `self.set_field(value)`. A nonstandard setter can be named explicitly. Fields that do not have a scalar representation, such as `TaskStatus`, are changed through named modifiers instead.

Point and bulk changes use the same pipeline:

```text
get ProverTask deadbeef | modify reset | stage

get ProverTask deadbeef | set retry_after_secs null | stage

scan ProverTask
| filter status == "pending" || status == "proving"
| modify abandon "operator cancelled"
| stage all
```

The programmatic API always requires a scan bound and may add a smaller `take 20` after filtering. A future text frontend may offer an explicit `all`, but it must remain a conspicuous opt-in. Cancellation or one modifier failure discards the new batch.

The session deliberately holds one `Box<dyn StagedWrite>` at a time. That write may contain one record or a bounded same-table batch, with stable before/after previews for every change. `abort` drops it. A point commit uses Sled's typed compare-and-swap; a batch commit checks every original value and applies every replacement in one Sled transaction. Neither path can overwrite a record that changed after staging, and a failed stale-value commit remains staged for inspection or abort.

Once a multi-row apply begins, cancellation waits rather than interrupting the transaction mid-write.

The current high-level database traits do not expose one arbitrary transaction spanning any set of Sled trees. A generic commit should therefore contain writes for only one table registration, which may supply its own atomic multi-row apply function. Cross-table or ordered mutations remain recipes until a production transaction coordinator exists. This avoids promising atomicity that the storage APIs do not provide.

## 6. Registration and use

For repo-owned values, derive on the value itself. The macro can see private fields and generate getter and setter dispatch without reconstructing the value. Foreign and consensus values that should not carry console annotations use a local adapter.

Console metadata is always compiled. The derive generates passive metadata and dispatch code: it opens no database, performs no I/O, and registers nothing globally. The console becomes active only when a caller explicitly builds a registry. This keeps normal Rust declarations free of repeated feature gates while retaining an auditable registration boundary.

The table still needs a thin registration because a value does not know which schema stores it, how its key is written, which broader operations are allowed, or which production API should persist it. The target declaration is:

```rust
#[derive(strata_db_console::ConsoleValue)]
pub struct TaskRecordData {
    #[console(get(
        scalar = "string",
        via = TaskRecordData::console_status
    ))]
    status: TaskStatus,

    #[console(get)]
    updated_at_secs: u64,

    #[console(get, set)]
    retry_after_secs: Option<u64>,

    #[console(get(
        name = "metadata_len",
        scalar = "u64",
        via = TaskRecordData::console_metadata_len
    ))]
    metadata: Option<Vec<u8>>,
}

impl TaskRecordData {
    fn console_status(&self) -> String {
        match &self.status {
            TaskStatus::Pending => "pending",
            TaskStatus::Proving { .. } => "proving",
            TaskStatus::Completed => "completed",
            TaskStatus::Blocked { .. } => "blocked",
            TaskStatus::TransientFailure { .. } => "transient_failure",
            TaskStatus::PermanentFailure { .. } => "permanent_failure",
        }
        .to_owned()
    }

    fn console_metadata_len(&self) -> Option<u64> {
        self.metadata.as_ref().map(|metadata| metadata.len() as u64)
    }
}

fn reset_task(value: &mut TaskRecordData) -> strata_db_console::ConsoleResult<()> {
    value.set_status(TaskStatus::Pending);
    value.set_retry_after_secs(None);
    Ok(())
}

fn abandon_task(
    value: &mut TaskRecordData,
    reason: &str,
) -> strata_db_console::ConsoleResult<()> {
    if value.status().is_terminal() {
        return Err(strata_db_console::ConsoleError::invalid_input(
            "modifier 'abandon'",
            "task is already terminal",
        ));
    }
    value.set_status(TaskStatus::PermanentFailure {
        error: reason.to_owned(),
    });
    Ok(())
}

#[derive(strata_db_console::ConsoleTable)]
#[console(
    name = "ProverTask",
    alias = "tasks",
    key = "bytes",
    schema = ProverTaskTree,
    value = TaskRecordData,
    adapter = SledConsoleTable,
    parse_key = parse_byte_key,
    render_key = render_byte_key,
    map_value = identity,
    unmap_value = identity,
    modifier(name = "reset", via = reset_task),
    modifier(
        name = "abandon",
        via = abandon_task,
        argument(name = "reason", scalar = "string")
    )
)]
struct ProverTaskConsoleTable(SledTree<ProverTaskTree>);

let mut registry = ConsoleRegistry::new();
registry.register(Arc::new(ProverTaskConsoleTable(tree.clone())))?;
```

`map_value` converts a decoded storage value into the registered console value. The optional `unmap_value` converts the edited value back to the schema's stored value and opts the table into point and bounded same-table writes. Omitting it leaves the table read-only, even if the value type exposes setters.

The four field forms are:

| Declaration | Meaning |
|---|---|
| `#[console(get)]` | Read the field directly. The console name and scalar type come from the field. |
| `#[console(get(via = path, ...))]` | Call a Rust method to project the field or value into a console scalar. `status` uses this because `TaskStatus` is not a scalar. |
| `#[console(get, set)]` | Read the field directly and write it only by calling the conventional `set_<field>` domain method. |
| `#[console(get, set(via = path))]` | Read the field directly and call the named domain setter instead of `set_<field>`. |

`via` always means “call this Rust function.” It does not bypass the type or access a field reflectively. For `get`, the function receives `&self` and returns the exposed scalar or optional scalar. For `set`, it receives `&mut self` and the parsed field value. `Option<T>` becomes nullable, and the scalar type is inferred for `bool`, `i64`, `u64`, `String`, and `Vec<u8>`.

`ConsoleValue` generates field metadata, scalar conversion, getter dispatch, and calls to declared domain setters. `ConsoleTable` generates the table trait implementation and owns broader modifiers and persistence. It delegates decoding, reads, scans, and writes to explicit storage hooks. The macros cannot infer domain semantics or cross-table invariants, and they never assign a field directly. Versioned registrations select the production decoder and fail closed on unsupported versions.

The read executor uses the same typed plans for direct tables and native views:

```rust
let registry = build_console_registry(&backend)?;
let executor = ReadExecutor::new(&registry);

let schema = executor.execute(ReadPlan::schema("ProverTask"))?;
let task = executor.execute(ReadPlan::get(
    "ProverTask",
    vec![ConsoleScalar::Bytes(task_key)],
))?;

let ReadOutput::Rows(tasks) = executor.execute(ReadPlan::scan_rev("tasks", 20)?)? else {
    unreachable!();
};
write_json_lines(tasks, std::io::stdout())?;

let block = executor.execute(ReadPlan::get(
    "OlBlock",
    vec![ConsoleScalar::Bytes(block_id)],
))?;
let sync = executor.execute(ReadPlan::get(
    "SyncInfo",
    vec![ConsoleScalar::U64(l1_reorg_safe_depth.into())],
))?;
```

The scan limit is part of `ScanPlan`, so a caller cannot accidentally construct an unbounded programmatic scan. `OlBlock` combines the stored block and status, while `SyncInfo` owns the reviewed cross-table read and accepts the reorg-safe depth explicitly.

Functional plans use a fixed expression tree rather than callbacks. The outer scan bound limits storage work; `take` limits matching output rows after filtering:

```rust
let pending = ScalarExpression::binary(
    BinaryOperator::Equal,
    ScalarExpression::field("status"),
    ScalarExpression::literal(ConsoleScalar::String("pending".into())),
);

let plan = PipelinePlan::scan_rev("tasks", 1_000)?
    .filter(pending.clone())
    .select(vec![
        Selection::new("key", ScalarExpression::Key),
        Selection::new("retry_after_secs", ScalarExpression::field("retry_after_secs")),
    ])
    .take(20)?;

let rows = PipelineExecutor::new(&registry).execute(plan)?;
let count = PipelineExecutor::new(&registry).execute(
    PipelinePlan::scan("tasks", 10_000)?
        .filter(pending.clone())
        .terminal(PipelineTerminal::Count),
)?;

let retry_sum = PipelineExecutor::new(&registry).execute(
    PipelinePlan::scan("tasks", 10_000)?
        .filter(ScalarExpression::binary(
            BinaryOperator::And,
            pending.clone(),
            ScalarExpression::binary(
                BinaryOperator::NotEqual,
                ScalarExpression::field("retry_after_secs"),
                ScalarExpression::literal(ConsoleScalar::Null),
            ),
        ))
        .terminal(PipelineTerminal::Sum(
            ScalarExpression::field("retry_after_secs"),
        )),
)?;
```

Writes use the same registry and expressions. A point setter stages one change. A filtered scan stages one atomic same-table batch; if any modifier fails while staging, or any stored value is stale at commit, no replacement is applied:

```rust
let mut writes = WriteSession::new(&registry);
let task_key = ConsoleScalar::Bytes(task_key);

let preview = writes.stage_set(
    "tasks",
    &task_key,
    "retry_after_secs",
    &ConsoleScalar::Null,
)?;
assert_eq!(preview.changes.len(), 1);
writes.commit()?;

let preview = writes.stage_modify_scan(
    PipelinePlan::scan("tasks", 1_000)?
        .filter(pending)
        .take(20)?,
    "abandon",
    &[ConsoleScalar::String("operator cancelled".into())],
)?;
assert!(preview.changes.len() <= 20);
writes.abort();
```

The resulting interaction uses only registered capabilities:

```text
$ strata-dbconsole --datadir data

db> schema ProverTask
key: hex bytes
getters: status, retry_after_secs, metadata_len, updated_at_secs
setters: retry_after_secs
modifiers: reset, abandon(reason)

db> get ProverTask deadbeef
status: proving; retry_after_secs: 1700000000; metadata_len: 32

db> scan_rev ProverTask
  | filter status == "pending" && metadata_len != null && metadata_len > 0
  | select key, status, metadata_len
  | take 20

db> scan ProverTask | filter status == "pending" | count
12

db> get ProverTask deadbeef | set retry_after_secs null | stage
staged 1 change

db> staged
ProverTask deadbeef: retry_after_secs 1700000000 -> null

db> commit
committed 1 change
```

## 7. Views, recipes, and Rhai

A native view prevents cross-table reads from leaking complexity into the expression language. Its output can use the value derive, while the view itself explicitly owns argument checking and the cross-table read:

```rust
#[derive(ConsoleValue)]
struct SyncInfoValue {
    #[console(get)]
    ol_tip_slot: u64,
    #[console(get)]
    current_epoch: u64,
    #[console(get)]
    finalized_epoch: u64,
}

struct SyncInfoView {
    // Narrow handles for the L1, OL block, OL state, and checkpoint stores.
}

impl ConsoleView for SyncInfoView {
    fn name(&self) -> &'static str { "SyncInfo" }
    fn arguments(&self) -> &'static [ArgumentDescriptor] { SYNC_INFO_ARGUMENTS }
    fn value_metadata(&self) -> &'static ValueMetadata {
        SyncInfoValue::value_metadata()
    }

    fn get(&self, arguments: &[ConsoleScalar]) -> ConsoleResult<Option<RecordHandle>> {
        // Parse l1_reorg_safe_depth, reuse authoritative cross-table reads,
        // and return a SyncInfoValue handle.
        todo!()
    }
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
- one writable table with `set`, `modify`, bounded or explicit-all staging, `staged`, `commit`, and `abort`;
- two representative native views, preferably `OlBlock` and `SyncInfo`;
- adapters that invoke existing high-risk repair commands as native recipes.

Migrate simple `get-*` and summary operations by registering their values and tables. Migrate composite reads to views while retaining compatibility wrappers for existing scripts and functional tests. Move safe record changes to typed modifiers only after their previews and validation match current behavior. Keep recovery logic native throughout the migration.

Do not reuse the `strata-dbtool` binary name initially. Coexistence makes output and behavior changes explicit and gives automation a deprecation window. Once the console covers the exercised workflows and compatibility wrappers are stable, `strata-dbtool` can become an alias or be retired.

Two implementation questions remain before the public APIs are fixed: which legacy porcelain and JSON fields require byte-for-byte stability, and whether generic deletion belongs in the MVP or follows after typed modification.

**Recommendation:** build the opaque-handle console with derived value metadata, thin table registrations, a fixed scalar expression language, native read views, and native recovery recipes. This design directly generalizes the common record workflows, accommodates all 30 current operations without pretending that cross-table recovery is generic, and provides a clear point at which an embedded language would become the more honest choice.
