import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { api, formatDuration, timeAgo, useEvents } from "../api";
import DagGraph from "../components/DagGraph";
import WorkflowBuilder from "../components/WorkflowBuilder";
import { Empty, RuntimeBadge, StatusPill, Tile } from "../components/ui";
import type { Run, TaskRun, Workflow, WorkflowSpec } from "../types";
import { computeMetrics, formatMs, HistoryBars } from "./Workflows";

/** Duration trend of recent runs, colored by outcome. */
function DurationTrend({ runs }: { runs: Run[] }) {
  const ordered = [...runs].reverse().slice(-30);
  const points = ordered.map((r) => ({
    run: r,
    ms:
      r.started_at && r.finished_at
        ? new Date(r.finished_at).getTime() - new Date(r.started_at).getTime()
        : 0,
  }));
  const max = Math.max(1, ...points.map((p) => p.ms));
  const W = 720;
  const H = 110;
  const slot = W / Math.max(12, points.length);
  const bw = Math.max(6, Math.min(26, slot - 5));
  const FILL: Record<string, string> = {
    completed: "#34d399",
    failed: "#f87171",
    running: "#38bdf8",
    cancelled: "#64748b",
    pending: "#64748b",
  };
  return (
    <svg viewBox={`0 0 ${W} ${H + 16}`} className="activity-chart" role="img" aria-label="Run duration trend">
      <line className="chart-grid" x1="0" x2={W} y1={H} y2={H} />
      {points.map((p, i) => {
        const h = Math.max(3, (p.ms / max) * (H - 10));
        const x = i * slot + (slot - bw) / 2;
        return (
          <rect key={p.run.id} x={x} y={H - h} width={bw} height={h} rx="3" fill={FILL[p.run.state] ?? "#64748b"}>
            <title>{`${p.run.state} · ${formatMs(p.ms)} · ${timeAgo(p.run.created_at)}`}</title>
          </rect>
        );
      })}
      <text className="chart-tick" x={0} y={H + 13}>
        older
      </text>
      <text className="chart-tick" x={W} y={H + 13} textAnchor="end">
        newest
      </text>
    </svg>
  );
}

function RunningProgress({ run, onClick }: { run: Run; onClick: () => void }) {
  const [tasks, setTasks] = useState<TaskRun[]>([]);

  const refresh = useCallback(() => {
    api
      .get<{ run: Run; tasks: TaskRun[] }>(`/api/runs/${run.id}`)
      .then((d) => setTasks(d.tasks))
      .catch(() => {});
  }, [run.id]);

  useEffect(refresh, [refresh]);
  useEvents((ev) => {
    if (ev.type === "task_updated") {
      setTasks((prev) => {
        const idx = prev.findIndex((t) => t.task_id === ev.task.task_id);
        if (idx === -1) return [...prev, ev.task];
        const next = [...prev];
        next[idx] = ev.task;
        return next;
      });
    }
  }, run.id);

  const done = tasks.filter((t) => t.state === "completed").length;
  const total = Math.max(1, tasks.length);
  const runningTask = tasks.find((t) => t.state === "running");
  return (
    <div className="running-banner" style={{ cursor: "pointer" }} onClick={onClick}>
      <StatusPill state="running" />
      <div className="progress-track">
        <div className="progress-fill" style={{ width: `${(done / total) * 100}%` }} />
      </div>
      <span style={{ color: "var(--ink)", fontWeight: 600, whiteSpace: "nowrap" }}>
        {done}/{total} tasks
      </span>
      {runningTask && (
        <span className="muted mono" style={{ fontSize: 12, whiteSpace: "nowrap" }}>
          ▸ {runningTask.task_id}
        </span>
      )}
    </div>
  );
}

export default function WorkflowDetail() {
  const { id } = useParams<{ id: string }>();
  const [workflow, setWorkflow] = useState<Workflow | null>(null);
  const [runs, setRuns] = useState<Run[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const navigate = useNavigate();

  const refresh = useCallback(() => {
    if (!id) return;
    api.get<Workflow>(`/api/workflows/${id}`).then(setWorkflow).catch((e) => setError(e.message));
    api.get<Run[]>(`/api/runs?workflow_id=${id}&limit=100`).then(setRuns).catch(() => {});
  }, [id]);

  useEffect(refresh, [refresh]);

  useEvents((ev) => {
    if (ev.type === "run_updated" && ev.run.workflow_id === id) {
      setRuns((prev) => {
        const idx = prev.findIndex((r) => r.id === ev.run.id);
        if (idx === -1) return [ev.run, ...prev];
        const next = [...prev];
        next[idx] = ev.run;
        return next;
      });
    }
  });

  const metrics = useMemo(() => computeMetrics(runs), [runs]);
  const successPct =
    metrics.completed + metrics.failed > 0
      ? Math.round((metrics.completed / (metrics.completed + metrics.failed)) * 100)
      : null;
  const activeRuns = runs.filter((r) => r.state === "running" || r.state === "pending");

  const save = async (spec: WorkflowSpec) => {
    const wf = await api.put<Workflow>(`/api/workflows/${id}`, spec);
    setWorkflow(wf);
    setEditing(false);
  };

  const trigger = async () => {
    setError(null);
    try {
      const run = await api.post<Run>(`/api/workflows/${id}/trigger`, { params: {} });
      navigate(`/runs/${run.id}`);
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const remove = async () => {
    if (!window.confirm(`Delete workflow "${workflow?.spec.name}"?`)) return;
    await api.delete(`/api/workflows/${id}`);
    navigate("/workflows");
  };

  if (!workflow) {
    return error ? <div className="error-banner">{error}</div> : <p className="muted">Loading…</p>;
  }

  return (
    <>
      <div className="crumbs">
        <Link to="/workflows">Workflows</Link> / {workflow.spec.name}
      </div>
      <div className="page-head">
        <div>
          <h1 style={{ display: "flex", alignItems: "center", gap: 12 }}>
            {workflow.spec.name}
            {metrics.last && <StatusPill state={metrics.last.state} />}
          </h1>
          <p>
            {workflow.spec.description || "No description."} ·{" "}
            {[...new Set(workflow.spec.tasks.map((t) => t.runtime))].map((r) => (
              <RuntimeBadge key={r} runtime={r} />
            ))}
          </p>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn danger" onClick={remove}>
            Delete
          </button>
          <button className="btn" onClick={() => setEditing((v) => !v)}>
            {editing ? "Close editor" : "✎ Edit"}
          </button>
          <button className="btn primary" onClick={trigger}>
            ▶ Trigger run
          </button>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {editing && (
        <div className="card" style={{ marginBottom: 20 }}>
          <div className="card-head">
            <h2>Edit workflow</h2>
          </div>
          <div className="card-body">
            <WorkflowBuilder initial={workflow.spec} submitLabel="Save" onSubmit={save} />
          </div>
        </div>
      )}

      {activeRuns.map((r) => (
        <RunningProgress key={r.id} run={r} onClick={() => navigate(`/runs/${r.id}`)} />
      ))}

      <div className="tiles">
        <Tile label="Total runs" value={metrics.total} />
        <Tile
          label="Success rate"
          value={successPct === null ? "—" : `${successPct}%`}
          sub={`${metrics.completed} ok / ${metrics.failed} failed`}
          tone={successPct === null ? undefined : successPct >= 80 ? "good" : "bad"}
        />
        <Tile
          label="Avg duration"
          value={metrics.avgMs != null ? formatMs(metrics.avgMs) : "—"}
          tone="accent"
        />
        <Tile
          label="Last run"
          value={metrics.last ? timeAgo(metrics.last.created_at) : "never"}
          sub={metrics.last ? `trigger: ${metrics.last.trigger}` : undefined}
        />
      </div>

      {runs.length > 0 && (
        <div className="card">
          <div className="card-head">
            <h2>Duration trend</h2>
            <HistoryBars history={metrics.history} />
          </div>
          <div className="card-body">
            <DurationTrend runs={runs} />
          </div>
        </div>
      )}

      <div className="card">
        <div className="card-head">
          <h2>Task graph</h2>
          <span className="muted" style={{ fontSize: 12 }}>
            {workflow.spec.tasks.length} tasks · max {workflow.spec.max_parallel_tasks} parallel
          </span>
        </div>
        <div className="card-body">
          <DagGraph tasks={workflow.spec.tasks} />
        </div>
      </div>

      <div className="card">
        <div className="card-head">
          <h2>Runs</h2>
        </div>
        {runs.length === 0 ? (
          <Empty title="Never run" hint="Trigger it to see run history." />
        ) : (
          <table>
            <thead>
              <tr>
                <th>Run</th>
                <th>State</th>
                <th>Trigger</th>
                <th className="num">Duration</th>
                <th className="num">Started</th>
              </tr>
            </thead>
            <tbody>
              {runs.slice(0, 25).map((r) => (
                <tr key={r.id} className="rowlink" onClick={() => navigate(`/runs/${r.id}`)}>
                  <td className="mono">{r.id.slice(0, 8)}</td>
                  <td>
                    <StatusPill state={r.state} />
                  </td>
                  <td>
                    <span className="trigger-chip">{r.trigger}</span>
                  </td>
                  <td className="num">{formatDuration(r.started_at, r.finished_at)}</td>
                  <td className="num muted">{timeAgo(r.created_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}
