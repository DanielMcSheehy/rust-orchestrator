# cortex-executor — worker protocol, pool, isolation

The most contract-heavy crate. Three things live here and must stay in sync:
the **wire protocol**, the **shims** that speak it, and the **pool/isolation**
machinery that spawns them.

## Wire protocol (THE contract)

One JSON request line on the worker's stdin per job:

```json
{"entry": "/path/to/job.py", "params": {...}, "inputs": {...}}
```

JSON-lines events on stdout, in order:

```json
{"type": "log",    "line": "..."}                     // 0..n, streamed live
{"type": "result", "value": <json>}                   // exactly one of these
{"type": "error",  "message": "...", "trace": "..."}  //   two ends the job
```

Rules:
- Exactly one terminal event (`result` XOR `error`) per job. The shim catches
  *everything* — a worker that reports a workload error is still healthy.
- After the terminal event the worker loops for the next stdin line (pooling
  depends on this). EOF on stdin ends the process.
- Any unparseable stdout line is a `Protocol` error and kills the worker.
- `entry` is the path *as the worker sees it* — host path in process mode,
  `/cortex/job/<file>` inside containers/microVMs. Never leak host paths into
  sandboxes.

**Changing the protocol means changing, in one commit:** `worker.py`,
`worker.mjs`, `parse_event`/`WorkerEvent`, `execute_pooled` + `execute_oneshot`,
and the tests.

## Shims (`shims/`)

- Embedded with `include_str!`, materialized to a tempdir at startup — the
  binary has no runtime file dependencies. Shims must stay **stdlib-only**
  (no pip/npm installs in workers).
- `cortex.py` / `cortex.mjs` are the in-task platform bindings (`import cortex`
  / `globalThis.cortex`): query/ingest/invoke against `CORTEX_API_URL`.
  Python bindings bypass HTTP proxies (`ProxyHandler({})`) — local API calls
  must never route through a corporate proxy. Keep both bindings' surfaces
  identical.
- Python: each job loads as a fresh module via `spec_from_file_location`;
  stdout is redirected per-job into log events (`REAL_STDOUT` carries the
  protocol). Node: `console.*` is rewired to log events; jobs import via
  unique file URLs (fresh module each job).

## Warm pool (`pool.rs`)

- Process isolation only. Container/microVM modes **never pool** — a fresh
  sandbox per task is their purpose.
- Checkout pops idle or spawns; one worker runs one job at a time.
- Check-in ONLY on a clean protocol finish (result or in-workload error).
  Timeout, protocol violation, or EOF ⇒ drop the worker (`kill_on_drop`).
- Workers retire after `CORTEX_WORKER_MAX_JOBS` (default 128) — this bounds
  Node's ESM module cache, which grows because every job is a unique URL.
  At most `CORTEX_WORKER_MAX_IDLE` (8) stay parked per runtime.
- ONE Node pool serves both JS and TS: workers start with
  `--experimental-strip-types` (a no-op for plain JS). Don't split the pools.
- A parked worker may have died: if the handoff write fails, retry once with
  a **fresh spawn** (`spawn_fresh`), not another possibly-stale pop.

## Isolation (`isolation.rs`)

- `plan()` is a pure function → `LaunchPlan {program, args, entry, container_name}`.
  All command construction lives there so it stays unit-testable without
  Docker. Keep it pure.
- Container/microVM: `run --rm -i --network none --read-only --tmpfs /tmp
  --pids-limit --memory --cpus`, shim + job dirs mounted read-only at
  `/cortex/shim` and `/cortex/job`. microVM = same invocation + `--runtime
  <vm-class>` (Kata/Firecracker), defaulting to `kata`.
- Timeouts kill the engine client AND `<engine> kill <container_name>` —
  SIGKILLing the client alone orphans the container/VM.

## Timeouts & zombies

- `tokio::time::timeout` wraps the read loop; the child is killed and (oneshot
  mode) reaped with a bounded `wait`. Every spawned process sets
  `kill_on_drop(true)` so server shutdown leaves no strays.
