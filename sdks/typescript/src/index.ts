/**
 * Cortex TypeScript SDK.
 *
 * Define tasks as plain functions, wire them into a flow, deploy against a
 * Cortex server, and stream live run events. Zero runtime dependencies —
 * built on `fetch`.
 *
 * ```ts
 * import { CortexClient, flow, task } from "@cortex/sdk";
 *
 * const extract = task("extract", async (params) => ({
 *   values: Array.from({ length: params.n as number }, (_, i) => i),
 * }));
 * const total = task(
 *   "total",
 *   async (_params, inputs) =>
 *     (inputs.extract as { values: number[] }).values.reduce((a, b) => a + b, 0),
 *   { dependsOn: [extract] },
 * );
 *
 * const client = new CortexClient("http://localhost:7420");
 * const wf = await client.deploy(flow("sum-pipeline", [extract, total], { params: { n: 100 } }));
 * const run = await client.trigger(wf.id, { wait: true });
 * ```
 *
 * Task handlers are serialized with `Function.prototype.toString()` and run
 * in an isolated worker process on the server — they must be self-contained
 * (no captured variables) and take `(params, inputs)`.
 */

export type Json =
  | null
  | boolean
  | number
  | string
  | Json[]
  | { [key: string]: Json };

export type RuntimeName = "python" | "typescript" | "javascript";
export type RunState = "pending" | "running" | "completed" | "failed" | "cancelled";

export type Handler = (
  params: Record<string, Json>,
  inputs: Record<string, Json>,
) => Json | undefined | Promise<Json | undefined>;

export interface TaskSpec {
  id: string;
  name?: string | null;
  runtime: RuntimeName;
  code: string;
  depends_on: string[];
  params: Record<string, Json>;
  timeout_secs: number;
  retries: number;
}

export interface WorkflowSpec {
  name: string;
  description?: string | null;
  params: Record<string, Json>;
  tasks: TaskSpec[];
  triggers: { every_secs?: number; on_ingest?: string };
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
  params: Json;
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
  result?: Json;
  error?: string | null;
  logs: string[];
  started_at?: string | null;
  finished_at?: string | null;
}

export interface CortexEvent {
  type: "run_updated" | "task_updated" | "log" | "ingested" | "function_invoked";
  ts: string;
  [key: string]: Json | undefined;
}

export interface TaskOptions {
  dependsOn?: Array<string | TaskSpec>;
  params?: Record<string, Json>;
  timeoutSecs?: number;
  retries?: number;
  /** Override the runtime; defaults to `javascript` for function handlers. */
  runtime?: RuntimeName;
}

const TERMINAL: RunState[] = ["completed", "failed", "cancelled"];

/**
 * Node 22+ runs TypeScript by replacing every type annotation with an
 * equal-width run of spaces (offset-preserving type stripping), so
 * `Function.prototype.toString()` returns source riddled with gaps:
 *
 *     async (params        , inputs         )              => {
 *
 * That garbage is what would land in the server and the console. Detect the
 * artifact (multi-space runs before `,` `)` `=` `{` `=>` never occur in
 * written code) and collapse the gaps — leading indentation and the inside
 * of strings, template literals, and comments are left untouched. Clean
 * sources pass through byte-for-byte.
 */
function collapseStrippedTypes(src: string): string {
  if (!/ {2,}[,)]|\) {2,}[={]| {2,}=>/.test(src)) return src;
  type Mode = "code" | "line" | "block" | "single" | "double" | "template";
  let mode: Mode = "code";
  const exprDepth: number[] = []; // brace depth of each nested `${ … }`
  let out = "";
  let atLineStart = true;
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    if (mode !== "code") {
      // Copy verbatim; watch only for the mode's exit (and template nesting).
      if (mode === "line" && ch === "\n") mode = "code";
      else if (mode === "block" && ch === "*" && src[i + 1] === "/") {
        out += "*/";
        i += 2;
        mode = "code";
        continue;
      } else if ((mode === "single" || mode === "double" || mode === "template") && ch === "\\") {
        out += src.slice(i, i + 2);
        i += 2;
        continue;
      } else if (mode === "single" && ch === "'") mode = "code";
      else if (mode === "double" && ch === '"') mode = "code";
      else if (mode === "template" && ch === "`") mode = "code";
      else if (mode === "template" && ch === "$" && src[i + 1] === "{") {
        out += "${";
        i += 2;
        exprDepth.push(0);
        mode = "code";
        continue;
      }
      out += ch;
      atLineStart = ch === "\n";
      i++;
      continue;
    }
    if (ch === "\n") {
      out += ch;
      atLineStart = true;
      i++;
      continue;
    }
    if (atLineStart && (ch === " " || ch === "\t")) {
      out += ch; // indentation is meaningful — keep it
      i++;
      continue;
    }
    atLineStart = false;
    const two = src.slice(i, i + 2);
    if (two === "//" || two === "/*") {
      mode = two === "//" ? "line" : "block";
      out += two;
      i += 2;
      continue;
    }
    if (ch === "'" || ch === '"' || ch === "`") {
      mode = ch === "'" ? "single" : ch === '"' ? "double" : "template";
      out += ch;
      i++;
      continue;
    }
    if (exprDepth.length > 0 && ch === "}" && exprDepth[exprDepth.length - 1] === 0) {
      exprDepth.pop();
      mode = "template";
      out += ch;
      i++;
      continue;
    }
    if (exprDepth.length > 0 && (ch === "{" || ch === "}")) {
      exprDepth[exprDepth.length - 1] += ch === "{" ? 1 : -1;
    }
    if (ch === " " && src[i + 1] === " ") {
      let j = i;
      while (src[j] === " ") j++;
      const next = src[j];
      // A stripped annotation's gap: vanish before closers/EOL, else one space.
      if (!(next === undefined || next === "\n" || next === "," || next === ")" || next === ";" || next === "(")) {
        out += " ";
      }
      i = j;
      continue;
    }
    out += ch;
    i++;
  }
  return out;
}

/**
 * Declare a task from a handler function (serialized and shipped to the
 * server) or from raw source code in any supported runtime.
 */
export function task(id: string, handler: Handler, options?: TaskOptions): TaskSpec;
export function task(
  id: string,
  source: { runtime: RuntimeName; code: string },
  options?: TaskOptions,
): TaskSpec;
export function task(
  id: string,
  impl: Handler | { runtime: RuntimeName; code: string },
  options: TaskOptions = {},
): TaskSpec {
  let runtime: RuntimeName;
  let code: string;
  if (typeof impl === "function") {
    runtime = options.runtime ?? "javascript";
    code = `export const handler = ${collapseStrippedTypes(impl.toString())};\n`;
  } else {
    runtime = impl.runtime;
    code = runtime === "python" ? impl.code : collapseStrippedTypes(impl.code);
  }
  return {
    id,
    runtime,
    code,
    depends_on: (options.dependsOn ?? []).map((d) => (typeof d === "string" ? d : d.id)),
    params: options.params ?? {},
    timeout_secs: options.timeoutSecs ?? 300,
    retries: options.retries ?? 0,
  };
}

export interface FlowOptions {
  description?: string;
  params?: Record<string, Json>;
  everySecs?: number;
  onIngest?: string;
  maxParallelTasks?: number;
}

/** Assemble tasks into a deployable workflow spec. */
export function flow(name: string, tasks: TaskSpec[], options: FlowOptions = {}): WorkflowSpec {
  const triggers: WorkflowSpec["triggers"] = {};
  if (options.everySecs) triggers.every_secs = options.everySecs;
  if (options.onIngest) triggers.on_ingest = options.onIngest;
  return {
    name,
    description: options.description ?? null,
    params: options.params ?? {},
    tasks,
    triggers,
    max_parallel_tasks: options.maxParallelTasks ?? 8,
  };
}

export class CortexError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(`HTTP ${status}: ${message}`);
  }
}

export class CortexClient {
  constructor(private baseUrl: string = "http://localhost:7420") {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const init: RequestInit = { method, headers: {} };
    if (body !== undefined) {
      if (typeof body === "string") {
        init.body = body;
        (init.headers as Record<string, string>)["content-type"] = "application/x-ndjson";
      } else {
        init.body = JSON.stringify(body);
        (init.headers as Record<string, string>)["content-type"] = "application/json";
      }
    }
    const res = await fetch(`${this.baseUrl}${path}`, init);
    if (!res.ok) {
      let detail = await res.text();
      try {
        detail = (JSON.parse(detail) as { error?: string }).error ?? detail;
      } catch {
        /* keep raw text */
      }
      throw new CortexError(res.status, detail);
    }
    if (res.status === 204) return undefined as T;
    return (await res.json()) as T;
  }

  // ── workflows & runs ───────────────────────────────────────────────────

  /** Create the workflow, or update it in place when the name already exists. */
  async deploy(spec: WorkflowSpec): Promise<Workflow> {
    const existing = (await this.listWorkflows()).find((w) => w.spec.name === spec.name);
    if (existing) {
      return this.request<Workflow>("PUT", `/api/workflows/${existing.id}`, spec);
    }
    return this.request<Workflow>("POST", "/api/workflows", spec);
  }

  listWorkflows(): Promise<Workflow[]> {
    return this.request("GET", "/api/workflows");
  }

  getWorkflow(id: string): Promise<Workflow> {
    return this.request("GET", `/api/workflows/${id}`);
  }

  deleteWorkflow(id: string): Promise<void> {
    return this.request("DELETE", `/api/workflows/${id}`);
  }

  async trigger(
    workflowId: string,
    options: { params?: Record<string, Json>; wait?: boolean; pollMs?: number } = {},
  ): Promise<Run> {
    const run = await this.request<Run>("POST", `/api/workflows/${workflowId}/trigger`, {
      params: options.params ?? {},
    });
    if (!options.wait) return run;
    const pollMs = options.pollMs ?? 500;
    for (;;) {
      const { run: current } = await this.getRun(run.id);
      if (TERMINAL.includes(current.state)) return current;
      await new Promise((r) => setTimeout(r, pollMs));
    }
  }

  listRuns(options: { workflowId?: string; limit?: number } = {}): Promise<Run[]> {
    const q = new URLSearchParams();
    if (options.workflowId) q.set("workflow_id", options.workflowId);
    q.set("limit", String(options.limit ?? 50));
    return this.request("GET", `/api/runs?${q}`);
  }

  getRun(id: string): Promise<{ run: Run; tasks: TaskRun[] }> {
    return this.request("GET", `/api/runs/${id}`);
  }

  // ── serverless functions ───────────────────────────────────────────────

  createFunction(spec: {
    name: string;
    code: string;
    runtime?: RuntimeName;
    description?: string;
    timeoutSecs?: number;
  }): Promise<Json> {
    const runtime = spec.runtime ?? "javascript";
    return this.request("POST", "/api/functions", {
      name: spec.name,
      code: runtime === "python" ? spec.code : collapseStrippedTypes(spec.code),
      runtime,
      description: spec.description ?? null,
      timeout_secs: spec.timeoutSecs ?? 300,
    });
  }

  invoke(
    name: string,
    params: Record<string, Json> = {},
  ): Promise<{ ok: boolean; result?: Json; error?: string; logs?: string[]; duration_ms: number }> {
    return this.request("POST", `/api/functions/${name}/invoke`, { params });
  }

  deleteFunction(name: string): Promise<void> {
    return this.request("DELETE", `/api/functions/${name}`);
  }

  // ── data ingestion ─────────────────────────────────────────────────────

  /** Stream records to a dataset as NDJSON. */
  ingest(dataset: string, records: Iterable<Json>): Promise<Json> {
    const body = Array.from(records, (r) => JSON.stringify(r)).join("\n") + "\n";
    return this.request("POST", `/api/ingest/${dataset}`, body);
  }

  listDatasets(): Promise<Json> {
    return this.request("GET", "/api/datasets");
  }

  /**
   * Run SQL against ingested datasets on the server's Rust (Polars)
   * dataframe engine, or against a registered external connector
   * (Postgres / ClickHouse / chDB).
   */
  async query(
    sql: string,
    options: { limit?: number; connector?: string } = {},
  ): Promise<Array<Record<string, Json>>> {
    const res = await this.request<{ rows: Array<Record<string, Json>> }>("POST", "/api/query", {
      sql,
      limit: options.limit ?? 10_000,
      connector: options.connector,
    });
    return res.rows;
  }

  /**
   * Run a code snippet on the server's warm worker pool. The code must
   * define/export `handler(params, inputs)`; `inputs` becomes the handler's
   * second argument.
   */
  execute(spec: {
    code: string;
    runtime?: RuntimeName;
    params?: Record<string, Json>;
    inputs?: Record<string, Json>;
    timeoutSecs?: number;
  }): Promise<{ ok: boolean; result?: Json; logs?: string[]; error?: string; duration_ms: number }> {
    const runtime = spec.runtime ?? "python";
    return this.request("POST", "/api/execute", {
      runtime,
      code: runtime === "python" ? spec.code : collapseStrippedTypes(spec.code),
      params: spec.params ?? {},
      inputs: spec.inputs ?? {},
      timeout_secs: spec.timeoutSecs ?? 120,
    });
  }

  stats(): Promise<Json> {
    return this.request("GET", "/api/stats");
  }

  // ── live events ────────────────────────────────────────────────────────

  private async openEventStream(
    path: string,
    signal?: AbortSignal,
  ): Promise<ReadableStream<Uint8Array>> {
    const res = await fetch(`${this.baseUrl}${path}`, { signal });
    if (!res.ok || !res.body) {
      throw new CortexError(res.status, "failed to open event stream");
    }
    return res.body;
  }

  private async *parseSse(body: ReadableStream<Uint8Array>): AsyncGenerator<CortexEvent> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return;
      buffer += decoder.decode(value, { stream: true });
      let idx: number;
      while ((idx = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, idx).trim();
        buffer = buffer.slice(idx + 1);
        if (line.startsWith("data:")) {
          yield JSON.parse(line.slice(5).trim()) as CortexEvent;
        }
      }
    }
  }

  /** Async-iterate server events (SSE). Pass a runId to scope to one run. */
  async *events(runId?: string, signal?: AbortSignal): AsyncGenerator<CortexEvent> {
    const path = runId ? `/api/runs/${runId}/events` : "/api/events";
    yield* this.parseSse(await this.openEventStream(path, signal));
  }

  /**
   * Stream one run's events until it reaches a terminal state.
   *
   * Attaches to the event stream *before* checking the run's current state,
   * so a run that already finished returns immediately instead of blocking
   * on events that were broadcast before we subscribed.
   */
  async *streamRun(runId: string): AsyncGenerator<CortexEvent> {
    const controller = new AbortController();
    const body = await this.openEventStream(`/api/runs/${runId}/events`, controller.signal);
    try {
      const { run } = await this.getRun(runId);
      if (TERMINAL.includes(run.state)) return;
      for await (const event of this.parseSse(body)) {
        yield event;
        if (
          event.type === "run_updated" &&
          TERMINAL.includes((event.run as unknown as Run).state)
        ) {
          return;
        }
      }
    } finally {
      controller.abort();
    }
  }
}
