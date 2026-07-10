# Console (React + Vite)

```bash
npm run dev        # :3001, proxies /api → :7420
npm run build      # tsc -b && vite build — the build IS the typecheck
```

## Design system — the rules that keep it looking like one product

- **CSS custom properties only** (`src/styles.css` `:root`). Never hardcode a
  hex in a component except SVG fills that mirror a token (state colors in
  charts). The palette: page `#070b15`, surfaces `#0d1424/#131c30/#1a2540`,
  ink `#f2f6fd/#b9c5da/#7e8ca6`, accent `#6d8dff`, violet `#a78bfa`, cyan
  `#22d3ee`, sky `#38bdf8`, good `#34d399`, warning `#fbbf24`, critical `#f87171`.
- **Pop is concentrated** in status, data, and interactive elements (pills,
  tiles, buttons, charts, DAG). Tables, labels, and body text stay quiet.
  That's the difference between vibrant and noisy — keep it.
- State → color is fixed everywhere: completed=good, running=sky (animated),
  failed=critical, pending/cancelled=muted. Charts are single-hue (sky) unless
  encoding state.
- Gradient (`--grad-brand`) appears on: wordmark, tile top edges, primary
  buttons, hero numbers. Don't spread it further.

## Patterns

- HTTP via `api.ts` helpers; errors surface the server's `{"error"}` message.
- **Live data**: subscribe with `useEvents((ev) => ...)` (SSE); merge updates
  into lists by id (find index → replace, prepend if new). Pages should not
  poll except run-completion waits.
- **CodeEditor vs CodeBlock split is load-bearing**: `CodeEditor.tsx` is a
  lazy wrapper (React.lazy) around `CodeMirrorEditor.tsx` so the ~229KB gzip
  CM6 chunk stays out of the main bundle (~82KB gzip). Read-only code uses
  Prism (`CodeBlock.tsx`). Never import `CodeMirrorEditor` statically from
  anything in the main graph.
- DAG rendering (`DagGraph.tsx`) mirrors the server's Kahn layering. Edge
  classes: `active` (upstream done → downstream running, animated marching
  dashes), `done` (hop completed), plain otherwise.
- Notebook cells are client-owned JSON (`NotebookCell` in `types.ts`); the
  server stores them opaquely. Cell execution = `/api/execute` (code) or
  `/api/query` (sql). Code cells chain: `cellInputs()` collects outputs of
  cells above (named cells keyed by name + `prev`) and sends them as
  `inputs`. Persist outputs with the document.
- `WorkflowBuilder.tsx` is the create/edit surface for workflows (Workflows
  page + WorkflowDetail edit): structured form ↔ JSON tab over the same
  `WorkflowSpec`; task cards address tasks by index so id edits don't
  remount (and drop focus on) the card.

## Verification rule

UI changes are not done until seen: build, serve via the release server
(`CORTEX_CONSOLE_DIST=console/dist`), drive with Playwright
(`executablePath: /opt/pw-browsers/chromium`, `waitUntil: "load"` — SSE keeps
`networkidle` from ever firing), screenshot, and look at it. Interactive
features (editors, live progress) get a behavioral check (type/click, assert),
not just a render.
