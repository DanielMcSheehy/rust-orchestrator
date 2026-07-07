# Development environment

Everything needed to hack on Cortex locally, and how the rebuild loop works.

## Prerequisites

| Tool | Version | Why |
| --- | --- | --- |
| Rust (rustup) | **stable, 1.80+** (do not use nightly-only features) | workspace builds; Polars 0.51 needs a recent stable, and 0.54+ needs nightly — that's why the pin exists |
| Node.js | **20+, 22+ recommended** | console build, TS SDK, JS/TS workers (Node 22's `--experimental-strip-types` runs TypeScript tasks) |
| Python | **3.10+** | Python workers and the Python SDK (stdlib only — no venv needed for the platform itself) |
| npm | ships with Node | console + TS SDK installs |
| Docker (optional) | any recent | only for `container`/`microvm` isolation modes and image builds |

```bash
# quick check
rustc --version && node --version && python3 --version
```

No database, broker, or other services are required — state is a SQLite file
the server creates under `CORTEX_DATA_DIR` (default `./data`).

## First run

```bash
git clone https://github.com/FormantIO/cortex && cd cortex

# 1. backend: build + run the server (API on :7420)
cargo run -p cortex-server                 # dev profile is fine for hacking

# 2. frontend: in a second terminal
cd console
npm install
npm run dev                                # console on :3001, proxies /api → :7420
```

Or run both with one script: `./scripts/dev.sh`.

To serve the *built* console from the server itself (what production does):

```bash
cd console && npm run build && cd ..
cargo run -p cortex-server                 # now http://localhost:7420 serves the UI
```

Smoke-test the loop end to end:

```bash
python3 examples/python_pipeline.py        # deploys a flow, ingests, streams a run
node examples/typescript_pipeline.mts      # needs sdks/typescript built (below)
```

## Rebuilding the Rust

The workspace is four crates: `cortex-core` → `cortex-store` / `cortex-executor`
→ `cortex-server` (the only binary).

```bash
cargo check --workspace                    # fastest signal while editing
cargo build -p cortex-server               # dev binary → target/debug/cortex-server
cargo build --release -p cortex-server     # optimized  → target/release/cortex-server
cargo test --workspace                     # all tests (see below)
cargo clippy --workspace --all-targets     # CI gates on -D warnings — run before pushing
```

Things worth knowing about the build:

- **First build is slow, rebuilds are fast.** Polars alone adds several
  minutes to a cold `--release` build. Incremental dev-profile rebuilds of
  `cortex-server` after a code change are typically seconds. Don't
  `cargo clean` unless you actually need to.
- **Benchmark with `--release` only.** The dev profile is 10–30× slower for
  the Polars/query paths; every number in the README was measured on release.
- **The worker shims are compiled in.** `crates/cortex-executor/shims/*`
  (`worker.py`, `worker.mjs`, `cortex.py`, `cortex.mjs`) are embedded with
  `include_str!` — editing a shim requires rebuilding `cortex-executor`
  (any `cargo build` picks it up; there is no runtime file to hot-swap).
- **Restart the server after rebuilding.** There is no hot reload for the
  binary. The console dev server (`npm run dev`) hot-reloads independently
  and just proxies `/api`, so frontend iteration doesn't need Rust rebuilds.
- **Version bumps:** Polars is pinned at 0.51 (stable-rustc ceiling) and the
  Rust edition at 2021 (`std::env::set_var` in `main.rs` is unsafe in 2024).
  Check `cargo check --workspace` on stable before touching either.

### Running the tests

```bash
cargo test --workspace                     # ~30 tests across the workspace
cargo test -p cortex-executor              # protocol/pool tests — spawn REAL python3/node
cargo test -p cortex-server                # orchestrator + Polars data-engine tests
```

The executor tests execute actual worker processes, so `python3` and `node`
must be on `PATH` — there are no mocks. If those tests fail, first suspect
your interpreters, not the code.

### Useful server env for development

```bash
RUST_LOG=debug cargo run -p cortex-server          # verbose tracing
CORTEX_DATA_DIR=/tmp/cortex-dev cargo run -p ...   # throwaway state
CORTEX_WORKER_POOL=0 cargo run -p ...              # disable warm pool (isolate pooling bugs)
CORTEX_PORT=8080 cargo run -p ...                  # move off :7420
```

Wipe state completely by deleting the data dir (SQLite file + dataset NDJSON
files live there and nowhere else).

## Rebuilding the frontend & SDKs

```bash
cd console && npm run build        # tsc -b && vite build — the build IS the typecheck
cd sdks/typescript && npm run build  # emits dist/ (the examples import from it)
# the Python SDK needs no build: pip install -e sdks/python (or just sys.path it)
```

Keep the CodeMirror bundle split intact: `CodeMirrorEditor.tsx` must only be
imported via the `React.lazy` wrapper in `CodeEditor.tsx` (see
`console/CLAUDE.md`).

## Docker

```bash
docker compose up --build          # multi-stage build: console → rust → runtime image
```

The image builds the console and the release binary from scratch — expect the
first build to take a while (Polars). Local iterative development is faster
outside Docker; use the image to verify packaging or for deployment
(`docker-compose.coolify.yml` for Coolify).

## Troubleshooting

| Symptom | Cause / fix |
| --- | --- |
| `error[E0658]: use of unstable library feature` in a polars crate | Polars got bumped past 0.51 — it needs nightly. Revert to `polars = "0.51"`. |
| executor tests fail with spawn errors | `python3` / `node` not on PATH (or too old — TS tasks need Node 22+). Override with `CORTEX_PYTHON_BIN` / `CORTEX_NODE_BIN`. |
| port 7420 in use | another cortex-server is running; `CORTEX_PORT=…` or kill it. |
| console shows stale UI against a rebuilt server | you're serving `console/dist` — rebuild it, or use `npm run dev` on :3001 during development. |
| `curl` to the API hangs through a corporate proxy | local calls must bypass proxies: `curl --noproxy '*' …` (the in-task bindings already do this). |
| first `cargo build --release` seems stuck | it's Polars; watch with `cargo build --release -p cortex-server -v`. Subsequent builds are incremental. |
