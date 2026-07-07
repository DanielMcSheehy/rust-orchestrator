import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, formatDuration, timeAgo, useEvents } from "../api";
import CodeEditor from "../components/CodeEditor";
import { Empty, RuntimeBadge, StatusPill } from "../components/ui";
import type { Run, Workflow, WorkflowSpec } from "../types";

const TEMPLATE = `{
  "name": "my-pipeline",
  "description": "Extract with Python, aggregate with TypeScript",
  "params": { "n": 100 },
  "tasks": [
    {
      "id": "extract",
      "runtime": "python",
      "code": "def handler(params, inputs):\\n    print('extracting', params['n'], 'rows')\\n    return {\\"values\\": list(range(params['n']))}\\n",
      "depends_on": []
    },
    {
      "id": "aggregate",
      "runtime": "typescript",
      "code": "export function handler(params: any, inputs: any) {\\n  const vs: number[] = inputs.extract.values;\\n  return { total: vs.reduce((a, b) => a + b, 0) };\\n}\\n",
      "depends_on": ["extract"]
    }
  ],
  "triggers": {}
}`;

export interface WorkflowMetrics {
  last?: Run;
  history: Run[]; // newest first
  total: number;
  completed: number;
  failed: number;
  avgMs: number | null;
}

export function computeMetrics(runs: Run[]): WorkflowMetrics {
  const completedRuns = runs.filter((r) => r.state === "completed");
  const durations = completedRuns
    .map((r) =>
      r.started_at && r.finished_at
        ? new Date(r.finished_at).getTime() - new Date(r.started_at).getTime()
        : null,
    )
    .filter((d): d is number => d !== null);
  return {
    last: runs[0],
    history: runs.slice(0, 12),
    total: runs.length,
    completed: completedRuns.length,
    failed: runs.filter((r) => r.state === "failed").length,
    avgMs: durations.length
      ? Math.round(durations.reduce((a, b) => a + b, 0) / durations.length)
      : null,
  };
}

export function HistoryBars({ history }: { history: Run[] }) {
  // Oldest → newest left-to-right, height scaled by duration.
  const ordered = [...history].reverse();
  const durations = ordered.map((r) =>
    r.started_at && r.finished_at
      ? new Date(r.finished_at).getTime() - new Date(r.started_at).getTime()
      : null,
  );
  const max = Math.max(1, ...durations.filter((d): d is number => d !== null));
  return (
    <span className="history-bars" title="Recent runs (height = duration)">
      {ordered.map((r, i) => {
        const h = durations[i] === null ? 8 : Math.max(6, (durations[i]! / max) * 22);
        return <span key={r.id} className={r.state} style={{ height: `${h}px` }} />;
      })}
    </span>
  );
}

export default function Workflows() {
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [runs, setRuns] = useState<Run[]>([]);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState(TEMPLATE);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  const refresh = useCallback(() => {
    api.get<Workflow[]>("/api/workflows").then(setWorkflows).catch(() => {});
    api.get<Run[]>("/api/runs?limit=500").then(setRuns).catch(() => {});
  }, []);

  useEffect(refresh, [refresh]);

  useEvents((ev) => {
    if (ev.type === "run_updated") {
      setRuns((prev) => {
        const idx = prev.findIndex((r) => r.id === ev.run.id);
        if (idx === -1) return [ev.run, ...prev];
        const next = [...prev];
        next[idx] = ev.run;
        return next;
      });
    }
  });

  const metricsByWorkflow = useMemo(() => {
    const grouped = new Map<string, Run[]>();
    for (const r of runs) {
      grouped.set(r.workflow_id, [...(grouped.get(r.workflow_id) ?? []), r]);
    }
    const out = new Map<string, WorkflowMetrics>();
    for (const [id, rs] of grouped) out.set(id, computeMetrics(rs));
    return out;
  }, [runs]);

  const create = async () => {
    setError(null);
    let spec: WorkflowSpec;
    try {
      spec = JSON.parse(draft) as WorkflowSpec;
    } catch (e) {
      setError(`invalid JSON: ${(e as Error).message}`);
      return;
    }
    try {
      const wf = await api.post<Workflow>("/api/workflows", spec);
      setCreating(false);
      navigate(`/workflows/${wf.id}`);
    } catch (e) {
      setError((e as Error).message);
    }
  };

  return (
    <>
      <div className="page-head">
        <div>
          <h1>Workflows</h1>
          <p>DAGs of Python and TypeScript tasks, orchestrated by the Rust core.</p>
        </div>
        <button className="btn primary" onClick={() => setCreating((v) => !v)}>
          {creating ? "Close" : "＋ New workflow"}
        </button>
      </div>

      {creating && (
        <div className="card" style={{ marginBottom: 20 }}>
          <div className="card-head">
            <h2>Define workflow (JSON)</h2>
            <button className="btn primary sm" onClick={create}>
              Create
            </button>
          </div>
          <div className="card-body">
            {error && <div className="error-banner">{error}</div>}
            <CodeEditor value={draft} language="json" minRows={16} onChange={setDraft} />
          </div>
        </div>
      )}

      <div className="card">
        {workflows.length === 0 ? (
          <Empty
            title="No workflows yet"
            hint="Create one here, or deploy from the Python / TypeScript SDK."
          />
        ) : (
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th>Last run</th>
                <th>History</th>
                <th className="num">Success</th>
                <th className="num">Avg time</th>
                <th>Triggers</th>
                <th>Runtimes</th>
              </tr>
            </thead>
            <tbody>
              {workflows.map((wf) => {
                const m = metricsByWorkflow.get(wf.id);
                const runtimes = [...new Set(wf.spec.tasks.map((t) => t.runtime))];
                const triggers = [
                  wf.spec.triggers.every_secs ? `every ${wf.spec.triggers.every_secs}s` : null,
                  wf.spec.triggers.on_ingest ? `ingest:${wf.spec.triggers.on_ingest}` : null,
                ].filter(Boolean);
                const successPct =
                  m && m.completed + m.failed > 0
                    ? Math.round((m.completed / (m.completed + m.failed)) * 100)
                    : null;
                return (
                  <tr key={wf.id} className="rowlink" onClick={() => navigate(`/workflows/${wf.id}`)}>
                    <td>
                      <div style={{ color: "var(--ink)", fontWeight: 600 }}>{wf.spec.name}</div>
                      <div className="muted" style={{ fontSize: 12 }}>
                        {wf.spec.tasks.length} task{wf.spec.tasks.length === 1 ? "" : "s"}
                        {wf.spec.description ? ` · ${wf.spec.description.slice(0, 48)}` : ""}
                      </div>
                    </td>
                    <td>
                      {m?.last ? (
                        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                          <StatusPill state={m.last.state} />
                          <span className="muted" style={{ fontSize: 12 }}>
                            {timeAgo(m.last.created_at)}
                          </span>
                        </div>
                      ) : (
                        <span className="muted">never ran</span>
                      )}
                    </td>
                    <td>{m ? <HistoryBars history={m.history} /> : <span className="muted">—</span>}</td>
                    <td className="num">
                      {successPct === null ? (
                        <span className="muted">—</span>
                      ) : (
                        <span
                          style={{
                            color: successPct >= 90 ? "var(--good)" : successPct >= 60 ? "var(--warning)" : "var(--critical)",
                            fontWeight: 650,
                          }}
                        >
                          {successPct}%
                        </span>
                      )}
                    </td>
                    <td className="num">
                      {m?.avgMs != null ? formatMs(m.avgMs) : <span className="muted">—</span>}
                    </td>
                    <td>
                      {triggers.length ? (
                        triggers.map((t) => (
                          <span key={t} className="trigger-chip" style={{ marginRight: 6 }}>
                            {t}
                          </span>
                        ))
                      ) : (
                        <span className="muted">manual</span>
                      )}
                    </td>
                    <td>
                      <span style={{ display: "inline-flex", gap: 6 }}>
                        {runtimes.map((r) => (
                          <RuntimeBadge key={r} runtime={r} />
                        ))}
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

export function formatMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}
