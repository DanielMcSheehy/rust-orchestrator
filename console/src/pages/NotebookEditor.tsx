import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import CodeEditor, { type CodeLanguage } from "../components/CodeEditor";
import Markdown from "../components/Markdown";
import ResultView from "../components/ResultView";
import { RuntimeBadge } from "../components/ui";
import type {
  CellOutput,
  ChartConfig,
  Connector,
  Notebook,
  NotebookCell,
  RuntimeName,
} from "../types";

const CODE_TEMPLATE: Record<RuntimeName, string> = {
  python: 'def handler(params, inputs):\n    return {"hello": "world"}\n',
  typescript: "export function handler(params: any) {\n  return { hello: 'world' };\n}\n",
  javascript: "export function handler(params) {\n  return { hello: 'world' };\n}\n",
};

let cellSeq = 0;
const newCellId = () => `c${Date.now().toString(36)}-${cellSeq++}`;

export default function NotebookEditor() {
  const { id } = useParams<{ id: string }>();
  const [notebook, setNotebook] = useState<Notebook | null>(null);
  const [cells, setCells] = useState<NotebookCell[]>([]);
  const [name, setName] = useState("");
  const [connectors, setConnectors] = useState<Connector[]>([]);
  const [running, setRunning] = useState<Set<string>>(new Set());
  const [dirty, setDirty] = useState(false);
  const [editing, setEditing] = useState<Set<string>>(new Set());
  const saveTimer = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => {
    if (!id) return;
    api.get<Notebook>(`/api/notebooks/${id}`).then((nb) => {
      setNotebook(nb);
      setName(nb.name);
      setCells(Array.isArray(nb.cells) ? nb.cells : []);
    });
    api.get<Connector[]>("/api/connectors").then(setConnectors).catch(() => {});
  }, [id]);

  // Debounced autosave.
  const scheduleSave = useCallback(
    (nextName: string, nextCells: NotebookCell[]) => {
      setDirty(true);
      clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(async () => {
        await api.put(`/api/notebooks/${id}`, { name: nextName, cells: nextCells });
        setDirty(false);
      }, 800);
    },
    [id],
  );

  const update = (next: NotebookCell[], nextName = name) => {
    setCells(next);
    scheduleSave(nextName, next);
  };

  const patchCell = (cellId: string, patch: Partial<NotebookCell>) => {
    update(cells.map((c) => (c.id === cellId ? { ...c, ...patch } : c)));
  };

  const addCell = (kind: NotebookCell["kind"], after?: string) => {
    const cell: NotebookCell =
      kind === "markdown"
        ? { id: newCellId(), kind, code: "## Notes\n", output: null }
        : kind === "sql"
          ? { id: newCellId(), kind, code: "SELECT 1 AS one", output: null }
          : { id: newCellId(), kind, runtime: "python", code: CODE_TEMPLATE.python, output: null };
    const idx = after ? cells.findIndex((c) => c.id === after) + 1 : cells.length;
    const next = [...cells.slice(0, idx), cell, ...cells.slice(idx)];
    setEditing((prev) => new Set(prev).add(cell.id));
    update(next);
  };

  const removeCell = (cellId: string) => update(cells.filter((c) => c.id !== cellId));

  const moveCell = (cellId: string, dir: -1 | 1) => {
    const idx = cells.findIndex((c) => c.id === cellId);
    const target = idx + dir;
    if (target < 0 || target >= cells.length) return;
    const next = [...cells];
    [next[idx], next[target]] = [next[target], next[idx]];
    update(next);
  };

  const runCell = async (cell: NotebookCell): Promise<NotebookCell> => {
    let output: CellOutput;
    try {
      if (cell.kind === "sql") {
        const res = await api.post<{ rows: Array<Record<string, unknown>>; elapsed_ms: number }>(
          "/api/query",
          { sql: cell.code, limit: 1000, connector: cell.connector || undefined },
        );
        output = { ok: true, rows: res.rows, elapsed_ms: res.elapsed_ms };
      } else {
        const res = await api.post<CellOutput & { duration_ms: number }>("/api/execute", {
          runtime: cell.runtime ?? "python",
          code: cell.code,
        });
        output = res.ok
          ? { ok: true, result: res.result, logs: res.logs, elapsed_ms: res.duration_ms }
          : { ok: false, error: res.error, logs: res.logs };
      }
    } catch (e) {
      output = { ok: false, error: (e as Error).message };
    }
    return { ...cell, output };
  };

  const run = async (cellId: string) => {
    const cell = cells.find((c) => c.id === cellId);
    if (!cell || cell.kind === "markdown") return;
    setRunning((prev) => new Set(prev).add(cellId));
    const done = await runCell(cell);
    setRunning((prev) => {
      const next = new Set(prev);
      next.delete(cellId);
      return next;
    });
    update(cells.map((c) => (c.id === cellId ? done : c)));
  };

  const runAll = async () => {
    let next = [...cells];
    for (let i = 0; i < next.length; i++) {
      if (next[i].kind === "markdown") continue;
      setRunning((prev) => new Set(prev).add(next[i].id));
      next[i] = await runCell(next[i]);
      setRunning((prev) => {
        const s = new Set(prev);
        s.delete(next[i].id);
        return s;
      });
      setCells([...next]);
    }
    update(next);
  };

  if (!notebook) return <p className="muted">Loading…</p>;

  return (
    <>
      <div className="crumbs">
        <Link to="/notebooks">Notebooks</Link> / {name}
      </div>
      <div className="page-head">
        <input
          className="notebook-title"
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            update(cells, e.target.value);
          }}
        />
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <span className="muted" style={{ fontSize: 12 }}>
            {dirty ? "saving…" : "saved"}
          </span>
          <button className="btn primary" onClick={runAll}>
            ▶ Run all
          </button>
        </div>
      </div>

      {cells.map((cell) => {
        const isRunning = running.has(cell.id);
        const isEditing = editing.has(cell.id) || cell.kind !== "markdown";
        return (
          <div className={`cell ${cell.kind}`} key={cell.id}>
            <div className="cell-gutter">
              {cell.kind !== "markdown" ? (
                <button
                  className="cell-run"
                  title="Run cell"
                  disabled={isRunning}
                  onClick={() => run(cell.id)}
                >
                  {isRunning ? "…" : "▶"}
                </button>
              ) : (
                <span className="cell-kind-dot" title="markdown">
                  ¶
                </span>
              )}
            </div>
            <div className="cell-main">
              <div className="cell-toolbar">
                <span className="cell-kind">{cell.kind}</span>
                {cell.kind === "code" && (
                  <select
                    value={cell.runtime ?? "python"}
                    onChange={(e) => {
                      const runtime = e.target.value as RuntimeName;
                      patchCell(cell.id, {
                        runtime,
                        code:
                          cell.code === CODE_TEMPLATE[cell.runtime ?? "python"]
                            ? CODE_TEMPLATE[runtime]
                            : cell.code,
                      });
                    }}
                  >
                    <option value="python">python</option>
                    <option value="typescript">typescript</option>
                    <option value="javascript">javascript</option>
                  </select>
                )}
                {cell.kind === "sql" && connectors.length > 0 && (
                  <select
                    value={cell.connector ?? ""}
                    onChange={(e) => patchCell(cell.id, { connector: e.target.value || undefined })}
                  >
                    <option value="">datasets (polars)</option>
                    {connectors.map((c) => (
                      <option key={c.name} value={c.name}>
                        {c.name} ({c.kind})
                      </option>
                    ))}
                  </select>
                )}
                {cell.output?.elapsed_ms !== undefined && (
                  <span className="muted">{cell.output.elapsed_ms}ms</span>
                )}
                <span className="cell-actions">
                  <button onClick={() => moveCell(cell.id, -1)} title="Move up">↑</button>
                  <button onClick={() => moveCell(cell.id, 1)} title="Move down">↓</button>
                  {cell.kind === "markdown" && (
                    <button
                      onClick={() =>
                        setEditing((prev) => {
                          const next = new Set(prev);
                          if (next.has(cell.id)) next.delete(cell.id);
                          else next.add(cell.id);
                          return next;
                        })
                      }
                    >
                      {editing.has(cell.id) ? "done" : "edit"}
                    </button>
                  )}
                  <button onClick={() => removeCell(cell.id)} title="Delete cell">✕</button>
                </span>
              </div>

              {cell.kind === "markdown" && !editing.has(cell.id) ? (
                <div onDoubleClick={() => setEditing((prev) => new Set(prev).add(cell.id))}>
                  <Markdown source={cell.code} />
                </div>
              ) : (
                isEditing && (
                  <CodeEditor
                    value={cell.code}
                    language={
                      (cell.kind === "sql"
                        ? "sql"
                        : cell.kind === "markdown"
                          ? "markdown"
                          : (cell.runtime ?? "python")) as CodeLanguage
                    }
                    minRows={3}
                    onChange={(code) => patchCell(cell.id, { code })}
                    onRun={() => {
                      if (cell.kind !== "markdown") run(cell.id);
                      else
                        setEditing((prev) => {
                          const next = new Set(prev);
                          next.delete(cell.id);
                          return next;
                        });
                    }}
                  />
                )
              )}

              {cell.output && (
                <div className="cell-output">
                  {cell.output.error && <div className="error-banner">{cell.output.error}</div>}
                  {cell.output.logs && cell.output.logs.length > 0 && (
                    <pre className="result-json cell-logs">{cell.output.logs.join("\n")}</pre>
                  )}
                  {cell.output.ok && (cell.output.rows ?? cell.output.result) !== undefined && (
                    <ResultView
                      value={cell.output.rows ?? cell.output.result}
                      chart={cell.chart ?? null}
                      onChart={(chart: ChartConfig | null) => patchCell(cell.id, { chart })}
                    />
                  )}
                </div>
              )}
            </div>
          </div>
        );
      })}

      <div className="add-cell-row">
        <button className="btn sm" onClick={() => addCell("code")}>
          + code
        </button>
        <button className="btn sm" onClick={() => addCell("sql")}>
          + sql
        </button>
        <button className="btn sm" onClick={() => addCell("markdown")}>
          + markdown
        </button>
        <span className="muted" style={{ marginLeft: 8, fontSize: 12 }}>
          ⌘⏎ runs a cell · code cells define <code>handler(params, inputs)</code> and can use{" "}
          <code>cortex.query()</code> / <code>cortex.ingest()</code>
        </span>
      </div>
      <div style={{ marginTop: 8 }}>
        <RuntimeBadge runtime="python" /> <RuntimeBadge runtime="typescript" />{" "}
        <RuntimeBadge runtime="javascript" />
      </div>
    </>
  );
}
