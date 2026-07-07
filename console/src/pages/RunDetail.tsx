import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api, formatDuration, useEvents } from "../api";
import { CodeBlock } from "../components/CodeEditor";
import DagGraph from "../components/DagGraph";
import LogStream, { type LogLine } from "../components/LogStream";
import { StatusPill } from "../components/ui";
import type { Run, RunState, TaskRun, Workflow } from "../types";

const STATE_FILL: Record<string, string> = {
  completed: "#34d399",
  failed: "#f87171",
  running: "#38bdf8",
  cancelled: "#64748b",
  pending: "#1a2540",
};

function TaskTimeline({ run, tasks }: { run: Run; tasks: TaskRun[] }) {
  const started = tasks
    .filter((t) => t.started_at)
    .sort((a, b) => (a.started_at! < b.started_at! ? -1 : 1));
  if (started.length === 0) return null;
  const t0 = new Date(run.started_at ?? started[0].started_at!).getTime();
  const tEnd = Math.max(
    ...started.map((t) => new Date(t.finished_at ?? new Date().toISOString()).getTime()),
    t0 + 1,
  );
  const span = tEnd - t0;
  const W = 760;
  const LABEL = 150;
  const ROW = 26;
  const H = started.length * ROW + 8;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="gantt" role="img" aria-label="Task timeline">
      {started.map((t, i) => {
        const s = new Date(t.started_at!).getTime();
        const e = new Date(t.finished_at ?? new Date().toISOString()).getTime();
        const x = LABEL + ((s - t0) / span) * (W - LABEL - 60);
        const w = Math.max(3, ((e - s) / span) * (W - LABEL - 60));
        const y = i * ROW + 6;
        return (
          <g key={t.task_id}>
            <text x={LABEL - 10} y={y + 12} textAnchor="end" className="gantt-label">
              {t.task_id.slice(0, 18)}
            </text>
            <line className="chart-grid" x1={LABEL} x2={W - 56} y1={y + 8} y2={y + 8} />
            <rect x={x} y={y} width={w} height={16} rx="4" fill={STATE_FILL[t.state] ?? "#383835"}>
              <title>{`${t.task_id}: ${t.state}, ${e - s}ms`}</title>
            </rect>
            <text x={x + w + 6} y={y + 12} className="gantt-ms">
              {e - s}ms
            </text>
          </g>
        );
      })}
    </svg>
  );
}

export default function RunDetail() {
  const { id } = useParams<{ id: string }>();
  const [run, setRun] = useState<Run | null>(null);
  const [tasks, setTasks] = useState<TaskRun[]>([]);
  const [workflow, setWorkflow] = useState<Workflow | null>(null);
  const [liveLogs, setLiveLogs] = useState<LogLine[]>([]);

  const refresh = useCallback(() => {
    if (!id) return;
    api
      .get<{ run: Run; tasks: TaskRun[] }>(`/api/runs/${id}`)
      .then(({ run, tasks }) => {
        setRun(run);
        setTasks(tasks);
        api.get<Workflow>(`/api/workflows/${run.workflow_id}`).then(setWorkflow).catch(() => {});
      })
      .catch(() => {});
  }, [id]);

  useEffect(refresh, [refresh]);

  useEvents((ev) => {
    if (ev.type === "run_updated") setRun(ev.run);
    if (ev.type === "task_updated") {
      setTasks((prev) => {
        const idx = prev.findIndex((t) => t.task_id === ev.task.task_id);
        if (idx === -1) return [...prev, ev.task];
        const next = [...prev];
        next[idx] = ev.task;
        return next;
      });
    }
    if (ev.type === "log") {
      setLiveLogs((prev) => [...prev, { ts: ev.ts, tag: ev.task_id, line: ev.line }].slice(-500));
    }
  }, id);

  if (!run) return <p className="muted">Loading…</p>;

  const states: Record<string, RunState> = Object.fromEntries(
    tasks.map((t) => [t.task_id, t.state]),
  );
  // Historical logs for finished runs where the live stream wasn't attached.
  const storedLogs: LogLine[] = tasks.flatMap((t) =>
    t.logs.map((line) => ({ ts: t.started_at ?? run.created_at, tag: t.task_id, line })),
  );
  const logs = liveLogs.length ? liveLogs : storedLogs;

  return (
    <>
      <div className="crumbs">
        <Link to="/runs">Runs</Link> / <span className="mono">{run.id.slice(0, 8)}</span>
      </div>
      <div className="page-head">
        <div>
          <h1 style={{ display: "flex", alignItems: "center", gap: 12 }}>
            {run.workflow_name} <StatusPill state={run.state} />
          </h1>
          <p>
            trigger <span className="trigger-chip">{run.trigger}</span> · duration{" "}
            {formatDuration(run.started_at, run.finished_at)} ·{" "}
            <Link to={`/workflows/${run.workflow_id}`}>view workflow</Link>
          </p>
        </div>
      </div>

      {run.error && <div className="error-banner">{run.error}</div>}

      {workflow && (
        <div className="card">
          <div className="card-head">
            <h2>Task graph</h2>
          </div>
          <div className="card-body">
            <DagGraph tasks={workflow.spec.tasks} states={states} />
          </div>
        </div>
      )}

      <div className="card">
        <div className="card-head">
          <h2>Timeline</h2>
        </div>
        <div className="card-body">
          <TaskTimeline run={run} tasks={tasks} />
        </div>
      </div>

      <div className="card">
        <div className="card-head">
          <h2>Tasks</h2>
        </div>
        {tasks.map((t) => (
          <details className="task-detail" key={t.task_id}>
            <summary>
              <div className="task-row">
                <span className="tname">{t.name}</span>
                <StatusPill state={t.state} />
                <span className="muted num">{formatDuration(t.started_at, t.finished_at)}</span>
                <span className="muted mono" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {t.error ?? (t.result !== null && t.result !== undefined ? JSON.stringify(t.result) : "")}
                </span>
              </div>
            </summary>
            <div className="task-detail-body">
              {t.error && <div className="error-banner">{t.error}</div>}
              {t.result !== null && t.result !== undefined && (
                <CodeBlock code={JSON.stringify(t.result, null, 2)} language="json" />
              )}
              {t.logs.length > 0 && (
                <pre className="result-json">{t.logs.join("\n")}</pre>
              )}
              <span className="muted" style={{ fontSize: 12 }}>
                {t.attempts} attempt{t.attempts === 1 ? "" : "s"}
              </span>
            </div>
          </details>
        ))}
      </div>

      <div className="card">
        <div className="card-head">
          <h2>Logs</h2>
          {run.state === "running" && (
            <span className="muted" style={{ fontSize: 12 }}>
              streaming live
            </span>
          )}
        </div>
        <LogStream lines={logs} />
      </div>
    </>
  );
}
