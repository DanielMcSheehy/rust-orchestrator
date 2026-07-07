# Cortex architecture

## Design goals

1. **Rust core, polyglot edge.** All orchestration, persistence, scheduling,
   and streaming logic lives in Rust for reliability and throughput. User
   workloads run where users write them: Python and TypeScript/JavaScript.
2. **Heavy data stays out of the control plane.** Ingestion streams NDJSON to
   disk chunk-by-chunk; workflows receive a *path* plus counters, not the
   payload. Task-to-task results flow as JSON and should be summaries or
   references, not raw datasets.
3. **Everything observable, live.** Every state transition and log line is a
   broadcast event, exposed over SSE. The console and both SDKs consume the
   same stream.
4. **Zero infrastructure by default.** One binary + SQLite (WAL). No broker,
   no external database. The `Store` is a thin layer so a Postgres backend
   can be added without touching the orchestrator.

## Execution model

```
trigger (manual | schedule | ingest | SDK)
   │
   ▼
launch_run: persist Run + TaskRuns (pending) ── broadcast run_updated
   │
   ▼
topo_layers(DAG) — Kahn's algorithm, validated at workflow creation
   │
   ├─ layer 0: [extract]                 ── all tasks in a layer run
   ├─ layer 1: [double, sum]                concurrently (bounded by
   └─ layer 2: [load]                       max_parallel_tasks)
        │
        ▼  per task
   executor.execute(runtime, code, params, inputs)
        │  1. write job file to scratch dir (job.py / job.ts / job.mjs)
        │  2. spawn worker shim (python3 worker.py | node worker.mjs)
        │  3. send {"entry", "params", "inputs"} on stdin
        │  4. stream {"type":"log"} events → broadcast + TaskRun.logs
        │  5. final {"type":"result"|"error"}; timeout ⇒ kill process
        ▼
   result value → inputs[task_id] for dependent tasks
```

Failure semantics: a task retries `retries` times; when it exhausts attempts,
the run fails, in-flight layer tasks finish, and downstream tasks are marked
`cancelled`. Task results/logs are persisted on every transition, so the
console shows historical runs identically to live ones.

## Worker protocol

One JSON request line on stdin, JSON-lines events on stdout:

```
→ {"entry": "/tmp/cortex-job-x/job.py", "params": {...}, "inputs": {...}}
← {"type": "log", "line": "..."}          (repeated, streamed live)
← {"type": "result", "value": <json>}     (exactly one, on success)
← {"type": "error", "message", "trace"}   (exactly one, on failure)
```

The shim loops over requests, so one worker process can serve many jobs.
In one-shot mode stderr is captured and tagged `[stderr]` into the log
stream; pooled workers inherit the server's stderr. The shims are embedded
in the `cortex-executor` binary (`include_str!`) and materialized to a temp
dir at startup — no runtime file dependencies.

## Warm worker pool

Interpreter startup (~30 ms CPython, ~60 ms Node, ~115 ms Node with TS
stripping) dwarfs typical task runtimes, so in `process` isolation mode the
executor pools workers (`pool.rs`):

- checkout pops an idle worker or spawns one; each worker runs one job at a
  time over the same stdin/stdout channel
- a clean finish (result *or* in-workload error) returns the worker to the
  pool; timeouts, protocol errors, and EOF kill it instead
- workers retire after `CORTEX_WORKER_MAX_JOBS` (default 128) jobs — this
  bounds interpreter-level accumulation such as Node's ESM module cache,
  which grows because every job is imported from a unique scratch path —
  and at most `CORTEX_WORKER_MAX_IDLE` (default 8) stay warm per runtime
- one Node pool serves both JS and TS (started with
  `--experimental-strip-types`, a no-op for plain JS)

Effect: serverless invoke p50 drops from ~36 ms (Python) / ~124 ms (TS) to
~1.2–1.6 ms; a 3-task mixed-runtime workflow completes end to end in ~7 ms.
Jobs share an interpreter but never a module: each job is loaded as a fresh
module from a unique path (warm `import`ed libraries are the upside, like
warm lambdas). Workloads needing hard per-task isolation should run in
`container` or `microvm` mode, which never pool.

TypeScript support uses Node 22's built-in type stripping
(`--experimental-strip-types`); no bundler or `tsc` in the hot path.

## Streaming

- `tokio::sync::broadcast` fans out `CortexEvent`s (run/task updates, logs,
  ingests, function invocations) to any number of SSE subscribers.
- Slow SSE consumers that miss buffer capacity skip dropped events rather
  than killing the stream.
- Ingestion (`POST /api/ingest/{dataset}`) consumes the request body as a
  byte stream, appending to `data/datasets/{dataset}.ndjson` and counting
  records without ever holding the batch in memory.

## Dataframe engine

Rather than inventing a dataframe library, the server embeds **Polars** — the
Rust dataframe engine — and exposes it everywhere (`cortex-server/src/data.rs`):

- `POST /api/query {"sql", "limit"}`: every ingested dataset registers as a
  SQL table (dashed names also aliased with underscores), so aggregations and
  cross-dataset joins run in-process. Scans are lazy — projection and
  predicate pushdown mean a `SELECT zone, AVG(value)` never materializes
  unused columns. Queries run on the blocking thread pool via
  `spawn_blocking`, so the async control plane stays responsive.
- Results are row-capped (`limit`, default 10k, max 200k) with an explicit
  `truncated` flag — the API returns summaries, not datasets.
- The same engine is reachable from the SDKs (`client.query(...)`) and from
  *inside* workloads: the worker shims materialize `cortex.py` / `cortex.mjs`
  bindings (`import cortex` in Python, the `cortex` global in JS/TS) that
  call back to the server over `CORTEX_API_URL`. Tasks aggregate millions of
  rows in Rust and pass small results downstream — no pandas/numpy needed in
  the worker environment.

## External engines (connectors)

`connectors.rs` routes `POST /api/query {"connector": name}` to registered
external sources: **Postgres** (native protocol, per-type JSON conversion),
**ClickHouse** (HTTP interface, `FORMAT JSONEachRow`), and **chDB**
(embedded ClickHouse executed *inside the Python worker runtime* — reusing
Cortex's own execution engine instead of linking libchdb into the server;
requires `pip install chdb` in the worker environment). Connector URLs are
stored in SQLite as plaintext — treat the store as sensitive.

## MCP (agents)

`mcp.rs` implements the Model Context Protocol's streamable-HTTP transport
at `POST /mcp` — hand-rolled JSON-RPC (initialize / ping / tools/list /
tools/call), stateless, no SDK dependency. Thirteen tools cover the whole
platform surface (workflows, runs, execute, query, ingest, functions,
notebooks), so an agent can compose pipelines, run them, and read results
without touching the REST API.

## Notebooks

Notebook documents (`/api/notebooks`) are stored as opaque JSON cell arrays;
the console owns the cell schema (markdown / code / sql + chart config) and
executes cells through `POST /api/execute` and `POST /api/query`. Execution
state lives in the saved document, so notebooks reopen with their results.

## Persistence

SQLite in WAL mode. Domain objects are stored as JSON documents with indexed
key columns (`state`, `workflow_id`, `created_at`) — the schema stays stable
while the model evolves, and reads never block the orchestrator's writes.

## Workload isolation

The executor builds a `LaunchPlan` per task execution
(`cortex-executor/src/isolation.rs`); the plan decides *what* gets spawned
while the protocol stays identical:

- **`process`** (default) — the interpreter is a direct child process.
  Trusted single-tenant deployments; lowest latency.
- **`container`** — `docker|podman run --rm -i` with `--network none`,
  `--read-only` rootfs (+ tmpfs `/tmp`), `--pids-limit`, `--memory`, and
  `--cpus` caps. The shim and job directories are bind-mounted read-only at
  `/cortex/shim` and `/cortex/job`; the request's `entry` path is rewritten
  to the guest mount.
- **`microvm`** — the same container invocation with an OCI **runtime class**
  that backs each container with a hardware-virtualized guest (Kata
  Containers, or Firecracker through `kata-fc`/firecracker-containerd).
  Each task gets its own kernel; a container-escape in user code lands
  inside a throwaway VM, not on the host. Defaults `--runtime kata`;
  override with `CORTEX_VM_RUNTIME`.

Timeout handling kills the engine client *and* issues `<engine> kill
<container-name>` so a wedged worker VM can't be orphaned. Host paths never
leak into sandboxed workers — the guest only sees the two read-only mounts.
