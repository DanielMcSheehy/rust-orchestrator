import { useMemo } from "react";
import type { RunState, TaskSpec } from "../types";

const NODE_W = 168;
const NODE_H = 52;
const GAP_X = 72;
const GAP_Y = 24;
const PAD = 16;

interface Placed {
  id: string;
  task: TaskSpec;
  x: number;
  y: number;
}

/** Layer tasks with Kahn's algorithm (mirrors the server's scheduler view). */
function layerTasks(tasks: TaskSpec[]): string[][] {
  const indegree = new Map<string, number>();
  const dependents = new Map<string, string[]>();
  for (const t of tasks) {
    indegree.set(t.id, t.depends_on.length);
    for (const dep of t.depends_on) {
      dependents.set(dep, [...(dependents.get(dep) ?? []), t.id]);
    }
  }
  const layers: string[][] = [];
  let frontier = tasks.filter((t) => indegree.get(t.id) === 0).map((t) => t.id);
  const seen = new Set<string>();
  while (frontier.length) {
    layers.push(frontier);
    const next: string[] = [];
    for (const id of frontier) {
      seen.add(id);
      for (const dep of dependents.get(id) ?? []) {
        indegree.set(dep, (indegree.get(dep) ?? 1) - 1);
        if (indegree.get(dep) === 0) next.push(dep);
      }
    }
    frontier = next;
  }
  // Cyclic leftovers (shouldn't exist — the server rejects them) get a layer.
  const leftovers = tasks.filter((t) => !seen.has(t.id)).map((t) => t.id);
  if (leftovers.length) layers.push(leftovers);
  return layers;
}

export default function DagGraph({
  tasks,
  states,
}: {
  tasks: TaskSpec[];
  states?: Record<string, RunState>;
}) {
  const { placed, edges, width, height } = useMemo(() => {
    const layers = layerTasks(tasks);
    const byId = new Map(tasks.map((t) => [t.id, t]));
    const placed = new Map<string, Placed>();
    const maxRows = Math.max(1, ...layers.map((l) => l.length));
    const height = PAD * 2 + maxRows * NODE_H + (maxRows - 1) * GAP_Y;
    layers.forEach((layer, col) => {
      const columnHeight = layer.length * NODE_H + (layer.length - 1) * GAP_Y;
      const yStart = (height - columnHeight) / 2;
      layer.forEach((id, row) => {
        placed.set(id, {
          id,
          task: byId.get(id)!,
          x: PAD + col * (NODE_W + GAP_X),
          y: yStart + row * (NODE_H + GAP_Y),
        });
      });
    });
    const edges = tasks.flatMap((t) =>
      t.depends_on
        .filter((dep) => placed.has(dep))
        .map((dep) => ({ from: placed.get(dep)!, to: placed.get(t.id)! })),
    );
    const width = PAD * 2 + layers.length * NODE_W + (layers.length - 1) * GAP_X;
    return { placed: [...placed.values()], edges, width, height };
  }, [tasks]);

  return (
    <div className="dag-wrap">
      <svg width={width} height={height} role="img" aria-label="Workflow task graph">
        {edges.map(({ from, to }, i) => {
          const x1 = from.x + NODE_W;
          const y1 = from.y + NODE_H / 2;
          const x2 = to.x;
          const y2 = to.y + NODE_H / 2;
          const mx = (x1 + x2) / 2;
          const upstream = states?.[from.id];
          const downstream = states?.[to.id];
          // `active`: results flowing into a running task — animated.
          // `done`: this hop completed. Plain otherwise.
          const cls =
            upstream === "completed" && downstream === "running"
              ? " active"
              : upstream === "completed" && downstream && downstream !== "pending"
                ? " done"
                : "";
          return (
            <path
              key={i}
              className={`dag-edge${cls}`}
              d={`M ${x1} ${y1} C ${mx} ${y1}, ${mx} ${y2}, ${x2} ${y2}`}
            />
          );
        })}
        {placed.map(({ id, task, x, y }) => (
          <g key={id} className={`dag-node ${states?.[id] ?? ""}`} transform={`translate(${x},${y})`}>
            <rect width={NODE_W} height={NODE_H} rx="8" />
            <text x="14" y="22">
              {(task.name ?? id).slice(0, 18)}
            </text>
            <text className="rt" x="14" y="39">
              {task.runtime}
              {task.depends_on.length ? ` ← ${task.depends_on.join(", ")}`.slice(0, 24) : ""}
            </text>
          </g>
        ))}
      </svg>
    </div>
  );
}
