import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, formatBytes, formatDuration, timeAgo, useEvents } from "../api";
import { Empty, StatusPill, Tile } from "../components/ui";
import type { CortexEvent, Run, Stats } from "../types";

function describe(ev: CortexEvent): string {
  switch (ev.type) {
    case "run_updated":
      return `run ${ev.run.workflow_name} → ${ev.run.state}`;
    case "task_updated":
      return `task ${ev.task.task_id} → ${ev.task.state}`;
    case "log":
      return `[${ev.task_id}] ${ev.line}`;
    case "ingested":
      return `ingested ${ev.records.toLocaleString()} records (${formatBytes(ev.bytes)}) into ${ev.dataset}`;
    case "function_invoked":
      return `function ${ev.name} ${ev.ok ? "succeeded" : "failed"} in ${ev.duration_ms}ms`;
  }
}

function ActivityChart({ runs }: { runs: Run[] }) {
  // Runs per hour over the last 24h, completed vs failed stacked.
  const buckets = Array.from({ length: 24 }, (_, i) => ({ ok: 0, failed: 0, hour: i }));
  const now = Date.now();
  for (const r of runs) {
    const age = now - new Date(r.created_at).getTime();
    if (age < 0 || age > 24 * 3600_000) continue;
    const idx = 23 - Math.floor(age / 3600_000);
    if (r.state === "failed") buckets[idx].failed++;
    else if (r.state === "completed") buckets[idx].ok++;
  }
  const max = Math.max(1, ...buckets.map((b) => b.ok + b.failed));
  const W = 720;
  const H = 120;
  const slot = W / 24;
  const bw = Math.max(4, slot - 4);
  return (
    <svg viewBox={`0 0 ${W} ${H + 18}`} className="activity-chart" role="img" aria-label="Runs in the last 24 hours">
      {buckets.map((b, i) => {
        const total = b.ok + b.failed;
        const hOk = (b.ok / max) * H;
        const hFail = (b.failed / max) * H;
        const x = i * slot + (slot - bw) / 2;
        const hourAgo = 23 - i;
        return (
          <g key={i}>
            {total === 0 && <rect x={x} y={H - 2} width={bw} height={2} rx="1" fill="var(--grid)" />}
            {b.ok > 0 && (
              <rect x={x} y={H - hOk} width={bw} height={Math.max(2, hOk)} rx="2" fill="#34d399">
                <title>{`${b.ok} completed, ${hourAgo}h ago`}</title>
              </rect>
            )}
            {b.failed > 0 && (
              <rect x={x} y={H - hOk - hFail - 2} width={bw} height={Math.max(2, hFail)} rx="2" fill="#f87171">
                <title>{`${b.failed} failed, ${hourAgo}h ago`}</title>
              </rect>
            )}
            {i % 4 === 0 && (
              <text x={x + bw / 2} y={H + 14} textAnchor="middle" className="chart-tick">
                {hourAgo === 0 ? "now" : `-${hourAgo}h`}
              </text>
            )}
          </g>
        );
      })}
    </svg>
  );
}

export default function Dashboard() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [runs, setRuns] = useState<Run[]>([]);
  const [feed, setFeed] = useState<CortexEvent[]>([]);
  const navigate = useNavigate();

  const refresh = useCallback(() => {
    api.get<Stats>("/api/stats").then(setStats).catch(() => {});
    api.get<Run[]>("/api/runs?limit=300").then(setRuns).catch(() => {});
  }, []);

  useEffect(refresh, [refresh]);

  useEvents((ev) => {
    setFeed((prev) => [ev, ...prev].slice(0, 60));
    if (ev.type === "run_updated" || ev.type === "ingested") refresh();
  });

  const successRate =
    stats && stats.runs_completed + stats.runs_failed > 0
      ? Math.round((stats.runs_completed / (stats.runs_completed + stats.runs_failed)) * 100)
      : null;

  return (
    <>
      <div className="page-head">
        <div>
          <h1>Dashboard</h1>
          <p>Live view of orchestration, workloads, and ingestion.</p>
        </div>
      </div>

      <div className="tiles">
        <Tile label="Workflows" value={stats?.workflows ?? "—"} />
        <Tile label="Active runs" value={stats?.runs_running ?? "—"} tone="accent" />
        <Tile
          label="Success rate"
          value={successRate === null ? "—" : `${successRate}%`}
          sub={stats ? `${stats.runs_completed} ok / ${stats.runs_failed} failed` : undefined}
          tone={successRate !== null && successRate < 80 ? "bad" : "good"}
        />
        <Tile label="Functions" value={stats?.functions ?? "—"} />
        <Tile
          label="Ingested"
          value={stats ? formatBytes(stats.bytes_ingested) : "—"}
          sub={stats ? `${stats.records_ingested.toLocaleString()} records` : undefined}
        />
      </div>

      <div className="card">
        <div className="card-head">
          <h2>Activity — last 24h</h2>
          <span className="legend">
            <span className="legend-swatch" style={{ background: "#34d399" }} /> completed
            <span className="legend-swatch" style={{ background: "#f87171", marginLeft: 12 }} /> failed
          </span>
        </div>
        <div className="card-body">
          <ActivityChart runs={runs} />
        </div>
      </div>

      <div className="card">
        <div className="card-head">
          <h2>Recent runs</h2>
        </div>
        {runs.length === 0 ? (
          <Empty title="No runs yet" hint="Trigger a workflow to see it here." />
        ) : (
          <table>
            <thead>
              <tr>
                <th>Workflow</th>
                <th>State</th>
                <th>Trigger</th>
                <th className="num">Duration</th>
                <th className="num">Started</th>
              </tr>
            </thead>
            <tbody>
              {runs.slice(0, 8).map((r) => (
                <tr key={r.id} className="rowlink" onClick={() => navigate(`/runs/${r.id}`)}>
                  <td>{r.workflow_name}</td>
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

      <div className="card">
        <div className="card-head">
          <h2>Live activity</h2>
        </div>
        <div className="feed">
          {feed.length === 0 ? (
            <Empty title="Quiet for now" hint="Events stream here in real time." />
          ) : (
            feed.map((ev, i) => (
              <div className="feed-item" key={i}>
                <span className="ts">{new Date(ev.ts).toLocaleTimeString()}</span>
                <span>{describe(ev)}</span>
              </div>
            ))
          )}
        </div>
      </div>
    </>
  );
}
