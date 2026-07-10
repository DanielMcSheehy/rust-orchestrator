// Structured workflow editor: name/triggers/params form + one card per task
// with a real code editor, dependency chips, and a live DAG preview. A JSON
// tab exposes the raw spec (params merging, retries, timeouts) for power
// users — the two views edit the same spec and stay in sync on tab switch.
import { useState } from "react";
import CodeEditor from "./CodeEditor";
import DagGraph from "./DagGraph";
import type { RuntimeName, TaskSpec, WorkflowSpec } from "../types";

const TASK_TEMPLATE: Record<RuntimeName, string> = {
  python:
    'def handler(params, inputs):\n    # inputs = {upstream_task_id: result}\n    return {"ok": True}\n',
  typescript:
    'export function handler(params: Record<string, unknown>, inputs: Record<string, unknown>) {\n  // inputs = {upstream_task_id: result}\n  return { ok: true };\n}\n',
  javascript:
    'export function handler(params, inputs) {\n  // inputs = {upstream_task_id: result}\n  return { ok: true };\n}\n',
};

const STARTER_TASKS: TaskSpec[] = [
  {
    id: "extract",
    name: null,
    runtime: "python",
    code: 'def handler(params, inputs):\n    print("extracting", params.get("n", 10), "rows")\n    return {"values": list(range(params.get("n", 10)))}\n',
    depends_on: [],
    params: {},
    timeout_secs: 300,
    retries: 0,
  },
  {
    id: "aggregate",
    name: null,
    runtime: "typescript",
    code: 'export function handler(params: Record<string, unknown>, inputs: { extract: { values: number[] } }) {\n  const vs = inputs.extract.values;\n  return { total: vs.reduce((a, b) => a + b, 0) };\n}\n',
    depends_on: ["extract"],
    params: {},
    timeout_secs: 300,
    retries: 0,
  },
];

function starterSpec(): WorkflowSpec {
  return {
    name: "my-pipeline",
    description: null,
    params: { n: 100 },
    tasks: structuredClone(STARTER_TASKS),
    triggers: {},
    max_parallel_tasks: 8,
  };
}

function newTask(existing: TaskSpec[]): TaskSpec {
  let n = existing.length + 1;
  let id = `task-${n}`;
  while (existing.some((t) => t.id === id)) id = `task-${++n}`;
  return {
    id,
    name: null,
    runtime: "python",
    code: TASK_TEMPLATE.python,
    depends_on: [],
    params: {},
    timeout_secs: 300,
    retries: 0,
  };
}

function validate(spec: WorkflowSpec): string | null {
  if (!spec.name.trim()) return "workflow needs a name";
  if (spec.tasks.length === 0) return "workflow needs at least one task";
  const ids = new Set<string>();
  for (const t of spec.tasks) {
    if (!t.id.trim()) return "every task needs an id";
    if (ids.has(t.id)) return `duplicate task id "${t.id}"`;
    ids.add(t.id);
  }
  for (const t of spec.tasks) {
    for (const dep of t.depends_on) {
      if (!ids.has(dep)) return `task "${t.id}" depends on unknown task "${dep}"`;
    }
  }
  return null;
}

export default function WorkflowBuilder({
  initial,
  submitLabel,
  onSubmit,
}: {
  initial?: WorkflowSpec;
  submitLabel: string;
  onSubmit: (spec: WorkflowSpec) => Promise<void>;
}) {
  const [spec, setSpec] = useState<WorkflowSpec>(() =>
    initial ? structuredClone(initial) : starterSpec(),
  );
  const [paramsDraft, setParamsDraft] = useState(() => JSON.stringify(spec.params ?? {}));
  const [mode, setMode] = useState<"builder" | "json">("builder");
  const [jsonDraft, setJsonDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  /** Builder state with the params JSON field folded in. Throws on bad JSON. */
  const composed = (): WorkflowSpec => {
    let params: unknown = {};
    if (paramsDraft.trim()) {
      try {
        params = JSON.parse(paramsDraft) as unknown;
      } catch (e) {
        throw new Error(`params is not valid JSON: ${(e as Error).message}`);
      }
    }
    return { ...spec, params };
  };

  const switchMode = (next: "builder" | "json") => {
    setError(null);
    if (next === mode) return;
    if (next === "json") {
      try {
        setJsonDraft(JSON.stringify(composed(), null, 2));
        setMode("json");
      } catch (e) {
        setError((e as Error).message);
      }
    } else {
      try {
        // Hand-written JSON may omit optional fields; normalize to the full
        // shape the builder form binds to.
        const parsed = JSON.parse(jsonDraft) as Partial<WorkflowSpec> & {
          tasks?: Partial<TaskSpec>[];
        };
        setSpec({
          name: parsed.name ?? "",
          description: parsed.description ?? null,
          params: parsed.params ?? {},
          triggers: parsed.triggers ?? {},
          max_parallel_tasks: parsed.max_parallel_tasks ?? 8,
          tasks: (parsed.tasks ?? []).map((t) => ({
            id: t.id ?? "",
            name: t.name ?? null,
            runtime: t.runtime ?? "python",
            code: t.code ?? "",
            depends_on: t.depends_on ?? [],
            params: t.params ?? {},
            timeout_secs: t.timeout_secs ?? 300,
            retries: t.retries ?? 0,
          })),
        });
        setParamsDraft(JSON.stringify(parsed.params ?? {}));
        setMode("builder");
      } catch (e) {
        setError(`invalid JSON: ${(e as Error).message}`);
      }
    }
  };

  // Tasks are addressed by index, not id: ids are freely editable text, and
  // keying on them would remount the card (and drop focus) on every keystroke.
  const patchTask = (idx: number, patch: Partial<TaskSpec>) => {
    setSpec((s) => ({
      ...s,
      tasks: s.tasks.map((t, i) => (i === idx ? { ...t, ...patch } : t)),
    }));
  };

  /** Rename a task and follow the id in every other task's depends_on. */
  const renameTask = (idx: number, nextId: string) => {
    setSpec((s) => {
      const oldId = s.tasks[idx].id;
      return {
        ...s,
        tasks: s.tasks.map((t, i) =>
          i === idx
            ? { ...t, id: nextId }
            : { ...t, depends_on: t.depends_on.map((d) => (d === oldId ? nextId : d)) },
        ),
      };
    });
  };

  const removeTask = (idx: number) => {
    setSpec((s) => {
      const id = s.tasks[idx].id;
      return {
        ...s,
        tasks: s.tasks
          .filter((_, i) => i !== idx)
          .map((t) => ({ ...t, depends_on: t.depends_on.filter((d) => d !== id) })),
      };
    });
  };

  const toggleDep = (idx: number, dep: string) => {
    setSpec((s) => ({
      ...s,
      tasks: s.tasks.map((t, i) =>
        i === idx
          ? {
              ...t,
              depends_on: t.depends_on.includes(dep)
                ? t.depends_on.filter((d) => d !== dep)
                : [...t.depends_on, dep],
            }
          : t,
      ),
    }));
  };

  const submit = async () => {
    setError(null);
    let final: WorkflowSpec;
    try {
      final = mode === "json" ? (JSON.parse(jsonDraft) as WorkflowSpec) : composed();
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    const invalid = validate(final);
    if (invalid) {
      setError(invalid);
      return;
    }
    setBusy(true);
    try {
      await onSubmit(final);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="wb">
      <div className="wb-head">
        <div className="wb-tabs" role="tablist">
          <button
            role="tab"
            aria-selected={mode === "builder"}
            className={mode === "builder" ? "on" : ""}
            onClick={() => switchMode("builder")}
          >
            Builder
          </button>
          <button
            role="tab"
            aria-selected={mode === "json"}
            className={mode === "json" ? "on" : ""}
            onClick={() => switchMode("json")}
          >
            JSON
          </button>
        </div>
        <button className="btn primary sm" disabled={busy} onClick={submit}>
          {busy ? "Saving…" : submitLabel}
        </button>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {mode === "json" ? (
        <CodeEditor value={jsonDraft} language="json" minRows={18} onChange={setJsonDraft} />
      ) : (
        <>
          <div className="form-row">
            <label className="field">
              <span>Name</span>
              <input
                type="text"
                value={spec.name}
                onChange={(e) => setSpec((s) => ({ ...s, name: e.target.value }))}
              />
            </label>
            <label className="field">
              <span>Description</span>
              <input
                type="text"
                placeholder="optional"
                value={spec.description ?? ""}
                onChange={(e) => setSpec((s) => ({ ...s, description: e.target.value || null }))}
              />
            </label>
          </div>
          <div className="form-row">
            <label className="field">
              <span>Run params (JSON)</span>
              <input
                type="text"
                className="mono"
                placeholder="{}"
                value={paramsDraft}
                onChange={(e) => setParamsDraft(e.target.value)}
              />
            </label>
            <label className="field">
              <span>Run every N seconds</span>
              <input
                type="text"
                inputMode="numeric"
                placeholder="never"
                value={spec.triggers.every_secs ?? ""}
                onChange={(e) => {
                  const n = parseInt(e.target.value, 10);
                  setSpec((s) => ({
                    ...s,
                    triggers: { ...s.triggers, every_secs: Number.isFinite(n) && n > 0 ? n : null },
                  }));
                }}
              />
            </label>
            <label className="field">
              <span>Run on ingest to dataset</span>
              <input
                type="text"
                placeholder="never"
                value={spec.triggers.on_ingest ?? ""}
                onChange={(e) =>
                  setSpec((s) => ({
                    ...s,
                    triggers: { ...s.triggers, on_ingest: e.target.value || null },
                  }))
                }
              />
            </label>
          </div>

          {spec.tasks.map((task, idx) => {
            const others = spec.tasks.filter((_, i) => i !== idx);
            return (
              <div className="task-card" key={idx}>
                <div className="task-card-head">
                  <input
                    className="task-id"
                    title="Task id — downstream tasks read this task's result as inputs[id]"
                    value={task.id}
                    onChange={(e) => renameTask(idx, e.target.value)}
                  />
                  <select
                    value={task.runtime}
                    onChange={(e) => {
                      const runtime = e.target.value as RuntimeName;
                      patchTask(idx, {
                        runtime,
                        code:
                          task.code === TASK_TEMPLATE[task.runtime]
                            ? TASK_TEMPLATE[runtime]
                            : task.code,
                      });
                    }}
                  >
                    <option value="python">python</option>
                    <option value="typescript">typescript</option>
                    <option value="javascript">javascript</option>
                  </select>
                  {others.length > 0 && (
                    <span className="dep-picker">
                      <span className="dep-label">after</span>
                      {others.map((o) => (
                        <button
                          key={o.id}
                          type="button"
                          className={`dep-chip${task.depends_on.includes(o.id) ? " on" : ""}`}
                          title={`Toggle dependency on "${o.id}"`}
                          onClick={() => toggleDep(idx, o.id)}
                        >
                          {o.id}
                        </button>
                      ))}
                    </span>
                  )}
                  <button
                    className="task-remove"
                    title="Remove task"
                    onClick={() => removeTask(idx)}
                  >
                    ✕
                  </button>
                </div>
                <CodeEditor
                  value={task.code}
                  language={task.runtime}
                  minRows={6}
                  onChange={(code) => patchTask(idx, { code })}
                />
              </div>
            );
          })}

          <div className="wb-actions">
            <button
              className="btn sm"
              onClick={() => setSpec((s) => ({ ...s, tasks: [...s.tasks, newTask(s.tasks)] }))}
            >
              + Add task
            </button>
            <span className="muted" style={{ fontSize: 12 }}>
              handlers run as <code>handler(params, inputs)</code> — <code>inputs</code> holds the
              results of the tasks marked “after”
            </span>
          </div>

          {spec.tasks.length > 1 && (
            <div className="wb-preview">
              <span className="wb-preview-label">Graph preview</span>
              <DagGraph tasks={spec.tasks} />
            </div>
          )}
        </>
      )}
    </div>
  );
}
