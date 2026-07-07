# Rust workspace

## Crate boundaries (dependency direction is one-way)

```
cortex-core  ←  cortex-store      (persistence)
             ←  cortex-executor   (worker processes)
             ←  cortex-server     (axum API, orchestrator, scheduler, data, mcp)
```

- `cortex-core` does **no I/O** — domain types, DAG algebra, events only.
  Anything shared between crates belongs here.
- `cortex-store` and `cortex-executor` never import each other.
- `cortex-server` is the only binary and the only crate that knows about HTTP.

## Conventions

- Dependencies come from `[workspace.dependencies]`; crates use `{ workspace = true }`.
- Errors: `thiserror` enum per crate (`StoreError`, `ExecError`, `DagError`);
  the server maps them to HTTP in `error.rs` (`NotFound → 404`, `DagError → 400`).
- Tests are colocated `#[cfg(test)]` modules; executor/server tests spawn real
  `python3`/`node` processes — keep them fast (<2s each) and self-contained.
- CI gates `cargo clippy --workspace --all-targets -- -D warnings`.

## Store (cortex-store)

- **JSON-document pattern**: full structs serialize into a `data TEXT` column;
  extra columns exist *only* for filtering/sorting (`state`, `workflow_id`,
  `created_at`, unique `name`). Add an indexed column only when a query needs
  it; otherwise evolve the JSON freely — that's the point.
- Schema changes are additive `CREATE TABLE/INDEX IF NOT EXISTS` in `SCHEMA`.
  There is no migration framework; never rename/drop in place.
- SQLite is WAL mode behind a `parking_lot::Mutex<Connection>` — calls are
  short and synchronous; do not hold the lock across `await`.

## Orchestrator invariants (cortex-server/src/orchestrator.rs)

- Execution walks `topo_layers` (Kahn); within a layer tasks run concurrently,
  bounded by `max_parallel_tasks` via a semaphore.
- `inputs` passed to a task = results of its `depends_on` only (not the whole
  run's results).
- `merge_params(base, overlay)`: objects merge key-by-key (overlay wins),
  `null` overlay keeps base, any other overlay replaces.
- Retries: `retries + 1` attempts; between attempts a log line announces the
  retry; workload stack traces land in `task_run.logs` as `[trace]` lines.
- On failure: current layer finishes, remaining tasks → `cancelled`, run →
  `failed` with `task \`id\` failed: message`.
- Every persist goes through `persist_run`/`persist_task` so store write +
  event emit stay paired. Don't write the store without emitting.

## Scheduler / triggers

- Interval scheduler keeps due-times **in memory**, seeded at `now + interval`
  on first sight — a restart waits one interval instead of stampeding.
- Ingest triggers fire after the payload is fully on disk; trigger params are
  `{dataset, records, bytes, path}` — tasks read the file from `path`, the
  payload never rides through the API.

## Data engine (cortex-server/src/data.rs)

- Every dataset registers as a SQL table + a `-`→`_` alias. Scans are lazy
  (`LazyJsonLineReader`); queries run under `spawn_blocking`, never on the
  async runtime.
- Results are row-capped (`limit`, clamp 1..=200_000) and fetch limit+1 to set
  `truncated` honestly. The API returns summaries, not datasets.

## MCP (cortex-server/src/mcp.rs)

- Hand-rolled JSON-RPC (streamable HTTP, stateless). Notifications (no `id`)
  → 202 empty. Tool results wrap JSON in `content[0].text`; tool failures set
  `isError: true` rather than JSON-RPC errors.
- New platform capability ⇒ add a tool here + list it in the README/docs.
