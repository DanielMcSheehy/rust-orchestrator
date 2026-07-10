# SDKs — Python (`cortex-sdk`) & TypeScript (`@cortex/sdk`)

## The two rules

1. **Zero runtime dependencies.** Python uses stdlib (`urllib`, `json`);
   TypeScript uses global `fetch` (Node 20+/browsers). A PR that adds a
   dependency to either SDK is wrong by default.
2. **Parity.** Every capability exists in both SDKs with the same semantics:
   deploy (upsert-by-name), trigger (+`wait`), runs, functions, invoke,
   ingest, `query(sql, connector?)`, `execute(code)`, events/streaming.
   New server endpoint ⇒ both SDKs + both READMEs in the same change.

## Contracts users depend on

- **`deploy` upserts by workflow name** — agents/scripts iterate by
  redeploying; never create duplicates.
- **Task code is self-contained**: it ships as source and runs in an isolated
  worker. Imports go inside the function body; no closure captures.
  - Python `@task`: source is extracted with `inspect.getsource`, decorator
    lines stripped, `handler = <fn>` appended. Decorated functions keep
    working locally and gain `.cortex_task`.
  - TS `task(id, handler)`: serialized via `Function.prototype.toString()` —
    document that closures don't survive. Raw source in another runtime goes
    through `task(id, {runtime, code})`.
  - **Stripped-type gap collapse**: Node 22+ type stripping replaces
    annotations with space runs, so `toString()` of a typed handler is full
    of gaps. `collapseStrippedTypes` (index.ts) detects the artifact and
    collapses it on every JS/TS code ingress (`task`, `createFunction`,
    `execute`); strings/templates/comments/indentation stay verbatim and
    clean code passes through byte-for-byte. Never apply it to Python.
- **`stream_run`/`streamRun` attach-then-check**: open the SSE connection
  *first*, then check run state, returning immediately if terminal. This
  closes the fast-run race (run finishes before subscription). Don't
  "simplify" the order.
- Handler signature everywhere: `handler(params, inputs)`; `inputs` maps
  upstream task ids to results.

## Layout

- `python/cortex_sdk/`: `client.py` (HTTP + streaming), `flow.py`
  (`Task`, `@task`, `Flow`). Tests ride the examples (`examples/*.py` run
  against a live server).
- `typescript/src/index.ts`: single-file client + builders; `npm run build`
  emits `dist/` (NodeNext, declaration files). Keep it one file until it hurts.
