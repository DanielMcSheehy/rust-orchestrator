// Shared renderer for query/execution results: table for row arrays with an
// optional chart view, JSON for everything else.
import { useMemo, useState } from "react";
import type { ChartConfig } from "../types";
import { CodeBlock } from "./CodeEditor";
import MiniChart from "./MiniChart";

export function rowsOf(value: unknown): Array<Record<string, unknown>> | null {
  if (Array.isArray(value) && value.length > 0 && value.every((v) => v && typeof v === "object" && !Array.isArray(v))) {
    return value as Array<Record<string, unknown>>;
  }
  return null;
}

export default function ResultView({
  value,
  chart,
  onChart,
}: {
  value: unknown;
  chart?: ChartConfig | null;
  /** When provided, the chart controls are shown and changes reported up. */
  onChart?: (c: ChartConfig | null) => void;
}) {
  const rows = rowsOf(value);
  const columns = useMemo(() => (rows ? Object.keys(rows[0]) : []), [rows]);
  const numericColumns = useMemo(
    () => (rows ? columns.filter((c) => rows.some((r) => Number.isFinite(Number(r[c])))) : []),
    [rows, columns],
  );
  const [localChart, setLocalChart] = useState<ChartConfig | null>(chart ?? null);
  const active = chart !== undefined ? chart : localChart;

  const setChart = (c: ChartConfig | null) => {
    setLocalChart(c);
    onChart?.(c);
  };

  if (!rows) {
    return <CodeBlock code={JSON.stringify(value, null, 2)} language="json" />;
  }

  return (
    <div>
      <div className="result-toolbar">
        <span className="muted">{rows.length.toLocaleString()} rows</span>
        <div className="seg">
          <button className={!active ? "on" : ""} onClick={() => setChart(null)}>
            table
          </button>
          <button
            className={active?.kind === "bar" ? "on" : ""}
            disabled={numericColumns.length === 0}
            onClick={() =>
              setChart({ kind: "bar", x: active?.x ?? columns[0], y: active?.y ?? numericColumns[0] })
            }
          >
            bar
          </button>
          <button
            className={active?.kind === "line" ? "on" : ""}
            disabled={numericColumns.length === 0}
            onClick={() =>
              setChart({ kind: "line", x: active?.x ?? columns[0], y: active?.y ?? numericColumns[0] })
            }
          >
            line
          </button>
        </div>
        {active && (
          <>
            <label className="inline-select">
              x
              <select value={active.x} onChange={(e) => setChart({ ...active, x: e.target.value })}>
                {columns.map((c) => (
                  <option key={c}>{c}</option>
                ))}
              </select>
            </label>
            <label className="inline-select">
              y
              <select value={active.y} onChange={(e) => setChart({ ...active, y: e.target.value })}>
                {numericColumns.map((c) => (
                  <option key={c}>{c}</option>
                ))}
              </select>
            </label>
          </>
        )}
      </div>
      {active ? (
        <MiniChart rows={rows} x={active.x} y={active.y} kind={active.kind} />
      ) : (
        <div className="result-table">
          <table>
            <thead>
              <tr>
                {columns.map((c) => (
                  <th key={c}>{c}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.slice(0, 200).map((row, i) => (
                <tr key={i}>
                  {columns.map((c) => (
                    <td key={c} className="mono">
                      {typeof row[c] === "object" ? JSON.stringify(row[c]) : String(row[c] ?? "")}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
