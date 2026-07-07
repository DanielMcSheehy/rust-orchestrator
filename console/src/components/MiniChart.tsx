// Minimal single-series chart (bar | line) for notebook cells and query
// results. One hue, thin marks, recessive grid — values read from an array
// of row objects.
import { useMemo } from "react";

const BLUE = "#38bdf8";
const W = 720;
const H = 260;
const PAD = { top: 14, right: 16, bottom: 42, left: 56 };

export default function MiniChart({
  rows,
  x,
  y,
  kind,
}: {
  rows: Array<Record<string, unknown>>;
  x: string;
  y: string;
  kind: "bar" | "line";
}) {
  const points = useMemo(
    () =>
      rows
        .map((r) => ({ label: String(r[x] ?? ""), value: Number(r[y]) }))
        .filter((p) => Number.isFinite(p.value))
        .slice(0, 120),
    [rows, x, y],
  );

  if (points.length === 0) {
    return <p className="muted">No numeric values in column “{y}”.</p>;
  }

  const innerW = W - PAD.left - PAD.right;
  const innerH = H - PAD.top - PAD.bottom;
  const max = Math.max(...points.map((p) => p.value), 0);
  const min = Math.min(...points.map((p) => p.value), 0);
  const span = max - min || 1;
  const yPos = (v: number) => PAD.top + innerH - ((v - min) / span) * innerH;
  const ticks = [min, min + span / 2, max];
  const labelEvery = Math.ceil(points.length / 10);

  return (
    <div className="chart-wrap">
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`${kind} chart of ${y} by ${x}`}>
        {ticks.map((t, i) => (
          <g key={i}>
            <line className="chart-grid" x1={PAD.left} x2={W - PAD.right} y1={yPos(t)} y2={yPos(t)} />
            <text className="chart-tick" x={PAD.left - 8} y={yPos(t) + 4} textAnchor="end">
              {formatTick(t)}
            </text>
          </g>
        ))}
        {kind === "bar" &&
          points.map((p, i) => {
            const slot = innerW / points.length;
            const bw = Math.max(2, Math.min(28, slot - 2));
            const xPos = PAD.left + i * slot + (slot - bw) / 2;
            const y0 = yPos(Math.max(0, min));
            const y1 = yPos(p.value);
            return (
              <rect
                key={i}
                x={xPos}
                y={Math.min(y0, y1)}
                width={bw}
                height={Math.max(1, Math.abs(y0 - y1))}
                rx="2"
                fill={BLUE}
              >
                <title>{`${p.label}: ${p.value}`}</title>
              </rect>
            );
          })}
        {kind === "line" && (
          <>
            <polyline
              fill="none"
              stroke={BLUE}
              strokeWidth="2"
              strokeLinejoin="round"
              points={points
                .map((p, i) => {
                  const xPos = PAD.left + (i / Math.max(1, points.length - 1)) * innerW;
                  return `${xPos},${yPos(p.value)}`;
                })
                .join(" ")}
            />
            {points.map((p, i) => {
              const xPos = PAD.left + (i / Math.max(1, points.length - 1)) * innerW;
              return (
                <circle key={i} cx={xPos} cy={yPos(p.value)} r="3.5" fill={BLUE}>
                  <title>{`${p.label}: ${p.value}`}</title>
                </circle>
              );
            })}
          </>
        )}
        {points.map((p, i) => {
          if (i % labelEvery !== 0) return null;
          const slot = innerW / points.length;
          const xPos =
            kind === "bar"
              ? PAD.left + i * slot + slot / 2
              : PAD.left + (i / Math.max(1, points.length - 1)) * innerW;
          return (
            <text key={i} className="chart-tick" x={xPos} y={H - PAD.bottom + 18} textAnchor="middle">
              {p.label.slice(0, 10)}
            </text>
          );
        })}
        <text className="chart-axis-label" x={PAD.left} y={H - 6}>
          {x}
        </text>
        <text className="chart-axis-label" x={PAD.left} y={PAD.top - 2}>
          {y}
        </text>
      </svg>
    </div>
  );
}

function formatTick(v: number): string {
  if (Math.abs(v) >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (Math.abs(v) >= 1_000) return `${(v / 1_000).toFixed(1)}k`;
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}
