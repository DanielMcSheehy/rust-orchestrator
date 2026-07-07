import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, formatDuration, timeAgo, useEvents } from "../api";
import { Empty, StatusPill } from "../components/ui";
import type { Run } from "../types";

export default function Runs() {
  const [runs, setRuns] = useState<Run[]>([]);
  const navigate = useNavigate();

  const refresh = useCallback(() => {
    api.get<Run[]>("/api/runs?limit=100").then(setRuns).catch(() => {});
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

  return (
    <>
      <div className="page-head">
        <div>
          <h1>Runs</h1>
          <p>Every workflow execution, updating live.</p>
        </div>
      </div>
      <div className="card">
        {runs.length === 0 ? (
          <Empty title="No runs yet" hint="Trigger a workflow to see it here." />
        ) : (
          <table>
            <thead>
              <tr>
                <th>Run</th>
                <th>Workflow</th>
                <th>State</th>
                <th>Trigger</th>
                <th className="num">Duration</th>
                <th className="num">Started</th>
              </tr>
            </thead>
            <tbody>
              {runs.map((r) => (
                <tr key={r.id} className="rowlink" onClick={() => navigate(`/runs/${r.id}`)}>
                  <td className="mono">{r.id.slice(0, 8)}</td>
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
    </>
  );
}
