# Cortex — agent guide

Rust-native orchestration platform for Python/TypeScript workloads: workflow
DAGs, serverless functions, streaming NDJSON ingestion, embedded SQL (Polars),
notebooks, and an MCP server — one binary, SQLite inside.

## Map

| Path | What lives there | Scoped guide |
| --- | --- | --- |
| `crates/` | Rust workspace: core → store/executor → server | `crates/CLAUDE.md` |
| `crates/cortex-executor/` | worker processes, wire protocol, pool, isolation | `crates/cortex-executor/CLAUDE.md` |
| `console/` | React + Vite UI | `console/CLAUDE.md` |
| `sdks/` | zero-dependency Python + TypeScript clients | `sdks/CLAUDE.md` |
| `site/` | landing page + docs (self-contained HTML, no build) | — |
| `docs/` | architecture notes, dev-environment guide (`development.md`), screenshots | — |
| `examples/` | runnable pipelines against a live server | — |

## Commands

```bash
cargo test --workspace                 # all Rust tests (spawns real python3/node workers)
cargo clippy --workspace --all-targets # CI gates on -D warnings
cargo run --release -p cortex-server   # API + console on :7420
cd console && npm run build            # tsc -b && vite build (build IS the typecheck)
cd sdks/typescript && npm run build
python3 examples/python_pipeline.py    # e2e smoke (needs running server)
```

Server env: `CORTEX_PORT` (7420), `CORTEX_DATA_DIR` (./data),
`CORTEX_CONSOLE_DIST` (./console/dist), `CORTEX_ISOLATION`
(process|container|microvm), `CORTEX_WORKER_POOL` (=0 disables),
`CORTEX_WORKER_MAX_IDLE` (8), `CORTEX_WORKER_MAX_JOBS` (128),
`CORTEX_PYTHON_BIN`/`CORTEX_NODE_BIN`, `CORTEX_API_URL` (injected for workers).

## Cross-cutting contracts (breaking any of these breaks users)

- **Run/TaskRun state machine**: `pending → running → completed|failed|cancelled`.
  Terminal states never transition again. A failed task fails the run; tasks
  never reached are marked `cancelled`.
- **Task handler signature**: `handler(params, inputs)` — `params` = run params
  merged with task params (overlay wins, `null` keeps base — see
  `orchestrator::merge_params`), `inputs` = `{upstream_task_id: result}`.
  This is public API across both SDKs, the shims, docs, and the console.
- **API error shape**: non-2xx responses are `{"error": "message"}`.
- **Event stream**: every state change / log line / ingest / invocation emits a
  `CortexEvent` (serde-tagged `type`, snake_case) on the broadcast bus; SSE
  endpoints are dumb subscribers. New observable behavior ⇒ new event variant.
- **Worker wire protocol**: JSON-lines over stdio, defined in
  `crates/cortex-executor/CLAUDE.md`. Changing it touches shims, executor,
  and pool simultaneously.
- **SDK parity**: any new HTTP endpoint gets a method in BOTH
  `sdks/python` and `sdks/typescript`, plus README examples.

## Rules

- **Verify like this session did**: unit tests + a live e2e (curl or SDK
  script against a running release server). UI changes get a real-browser
  Playwright screenshot before they're called done.
- **No authentication exists.** Trusted single-tenant by design. Never imply
  otherwise in docs/UI; keep the caveat in README/deploy docs when touching them.
- **Polars is pinned to 0.51** — 0.54+ needs nightly rustc features. Don't bump
  without checking `cargo check` on stable.
- **Rust edition stays 2021** — `main.rs` uses `std::env::set_var` (unsafe in 2024).
- **SDKs stay zero-dependency** (Python stdlib / global fetch only).
- **Dataset & function & connector names**: `[a-zA-Z0-9_-]{1,64}` (`is_safe_name`).
- Benchmarks quoted anywhere (README, landing page) must be *measured*, with
  the environment stated. Update `site/index.html` stats if they change.
