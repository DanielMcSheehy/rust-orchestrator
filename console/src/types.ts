export type RunState = "pending" | "running" | "completed" | "failed" | "cancelled";
export type RuntimeName = "python" | "typescript" | "javascript";

export interface TaskSpec {
  id: string;
  name?: string | null;
  runtime: RuntimeName;
  code: string;
  depends_on: string[];
  params: unknown;
  timeout_secs: number;
  retries: number;
}

export interface WorkflowSpec {
  name: string;
  description?: string | null;
  params: unknown;
  tasks: TaskSpec[];
  triggers: { every_secs?: number | null; on_ingest?: string | null };
  max_parallel_tasks: number;
}

export interface Workflow {
  id: string;
  spec: WorkflowSpec;
  created_at: string;
  updated_at: string;
}

export interface Run {
  id: string;
  workflow_id: string;
  workflow_name: string;
  state: RunState;
  params: unknown;
  trigger: string;
  error?: string | null;
  created_at: string;
  started_at?: string | null;
  finished_at?: string | null;
}

export interface TaskRun {
  id: string;
  run_id: string;
  task_id: string;
  name: string;
  state: RunState;
  attempts: number;
  result?: unknown;
  error?: string | null;
  logs: string[];
  started_at?: string | null;
  finished_at?: string | null;
}

export interface CortexFunction {
  id: string;
  spec: {
    name: string;
    description?: string | null;
    runtime: RuntimeName;
    code: string;
    timeout_secs: number;
  };
  invocations: number;
  created_at: string;
  updated_at: string;
}

export interface Dataset {
  name: string;
  records: number;
  bytes: number;
  created_at: string;
  updated_at: string;
}

export interface Stats {
  workflows: number;
  functions: number;
  datasets: number;
  runs_total: number;
  runs_running: number;
  runs_completed: number;
  runs_failed: number;
  records_ingested: number;
  bytes_ingested: number;
}

export type ConnectorKind = "postgres" | "clickhouse" | "chdb";

export interface Connector {
  name: string;
  kind: ConnectorKind;
  url: string;
  created_at: string;
}

export type CellKind = "markdown" | "code" | "sql";

export interface NotebookCell {
  id: string;
  kind: CellKind;
  runtime?: RuntimeName;
  connector?: string;
  code: string;
  /** Persisted last output so notebooks re-open with their results. */
  output?: CellOutput | null;
  chart?: ChartConfig | null;
}

export interface CellOutput {
  ok: boolean;
  result?: unknown;
  rows?: Array<Record<string, unknown>>;
  logs?: string[];
  error?: string;
  elapsed_ms?: number;
}

export interface ChartConfig {
  kind: "bar" | "line";
  x: string;
  y: string;
}

export interface Notebook {
  id: string;
  name: string;
  cells: NotebookCell[] | null;
  created_at: string;
  updated_at: string;
}

export type CortexEvent =
  | { type: "run_updated"; ts: string; run: Run }
  | { type: "task_updated"; ts: string; task: TaskRun }
  | { type: "log"; ts: string; run_id: string; task_id: string; line: string }
  | { type: "ingested"; ts: string; dataset: string; records: number; bytes: number }
  | { type: "function_invoked"; ts: string; name: string; ok: boolean; duration_ms: number };
