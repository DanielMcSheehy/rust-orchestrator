import type { ReactNode } from "react";
import type { RunState, RuntimeName } from "../types";

export function StatusPill({ state }: { state: RunState }) {
  return <span className={`pill ${state}`}>{state}</span>;
}

export function RuntimeBadge({ runtime }: { runtime: RuntimeName }) {
  const label = { python: "py", typescript: "ts", javascript: "js" }[runtime];
  return <span className={`runtime-badge ${runtime}`}>{label}</span>;
}

export function Tile({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: ReactNode;
  sub?: string;
  tone?: "accent" | "good" | "bad";
}) {
  return (
    <div className={`tile${tone ? ` ${tone}` : ""}`}>
      <div className="label">{label}</div>
      <div className="value">{value}</div>
      {sub && <div className="sub">{sub}</div>}
    </div>
  );
}

export function Empty({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="empty">
      <div>{title}</div>
      {hint && <div className="hint">{hint}</div>}
    </div>
  );
}
