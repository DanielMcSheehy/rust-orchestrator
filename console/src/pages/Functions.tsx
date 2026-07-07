import { useCallback, useEffect, useState } from "react";
import { api, timeAgo } from "../api";
import CodeEditor, { CodeBlock } from "../components/CodeEditor";
import { Empty, RuntimeBadge } from "../components/ui";
import type { CortexFunction, RuntimeName } from "../types";

const TEMPLATES: Record<RuntimeName, string> = {
  python: `def handler(params, inputs):\n    name = params.get("name", "world")\n    return {"greeting": f"hello {name}"}\n`,
  typescript: `export function handler(params: { name?: string }) {\n  return { greeting: \`hello \${params.name ?? "world"}\` };\n}\n`,
  javascript: `export function handler(params) {\n  return { greeting: \`hello \${params.name ?? "world"}\` };\n}\n`,
};

interface InvokeResult {
  ok: boolean;
  result?: unknown;
  error?: string;
  logs?: string[];
  duration_ms: number;
}

export default function Functions() {
  const [functions, setFunctions] = useState<CortexFunction[]>([]);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("hello");
  const [runtime, setRuntime] = useState<RuntimeName>("python");
  const [code, setCode] = useState(TEMPLATES.python);
  const [error, setError] = useState<string | null>(null);
  const [invokeParams, setInvokeParams] = useState<Record<string, string>>({});
  const [results, setResults] = useState<Record<string, InvokeResult>>({});
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.get<CortexFunction[]>("/api/functions").then(setFunctions).catch(() => {});
  }, []);

  useEffect(refresh, [refresh]);

  const create = async () => {
    setError(null);
    try {
      await api.post("/api/functions", {
        name,
        runtime,
        code,
        timeout_secs: 300,
      });
      setCreating(false);
      refresh();
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const invoke = async (fname: string) => {
    setBusy(fname);
    try {
      const params = invokeParams[fname] ? JSON.parse(invokeParams[fname]) : {};
      const res = await api.post<InvokeResult>(`/api/functions/${fname}/invoke`, { params });
      setResults((prev) => ({ ...prev, [fname]: res }));
      refresh();
    } catch (e) {
      setResults((prev) => ({
        ...prev,
        [fname]: { ok: false, error: (e as Error).message, duration_ms: 0 },
      }));
    } finally {
      setBusy(null);
    }
  };

  const remove = async (fname: string) => {
    if (!window.confirm(`Delete function "${fname}"?`)) return;
    await api.delete(`/api/functions/${fname}`);
    refresh();
  };

  return (
    <>
      <div className="page-head">
        <div>
          <h1>Functions</h1>
          <p>Serverless handlers in Python or TypeScript — deploy once, invoke over HTTP.</p>
        </div>
        <button className="btn primary" onClick={() => setCreating((v) => !v)}>
          {creating ? "Close" : "New function"}
        </button>
      </div>

      {creating && (
        <div className="card" style={{ marginBottom: 20 }}>
          <div className="card-head">
            <h2>Deploy function</h2>
            <button className="btn primary sm" onClick={create}>
              Deploy
            </button>
          </div>
          <div className="card-body">
            {error && <div className="error-banner">{error}</div>}
            <div className="form-row">
              <label className="field">
                <span>Name</span>
                <input type="text" value={name} onChange={(e) => setName(e.target.value)} />
              </label>
              <label className="field">
                <span>Runtime</span>
                <select
                  value={runtime}
                  onChange={(e) => {
                    const rt = e.target.value as RuntimeName;
                    setRuntime(rt);
                    setCode(TEMPLATES[rt]);
                  }}
                >
                  <option value="python">Python</option>
                  <option value="typescript">TypeScript</option>
                  <option value="javascript">JavaScript</option>
                </select>
              </label>
            </div>
            <label className="field">
              <span>Handler code</span>
              <CodeEditor value={code} language={runtime} minRows={8} onChange={setCode} />
            </label>
          </div>
        </div>
      )}

      {functions.length === 0 ? (
        <div className="card">
          <Empty title="No functions deployed" hint="Deploy one here or via the SDKs." />
        </div>
      ) : (
        functions.map((f) => {
          const res = results[f.spec.name];
          return (
            <div className="card" key={f.id}>
              <div className="card-head">
                <h2 style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  {f.spec.name} <RuntimeBadge runtime={f.spec.runtime} />
                </h2>
                <span className="muted" style={{ fontSize: 12 }}>
                  {f.invocations} invocation{f.invocations === 1 ? "" : "s"} · updated{" "}
                  {timeAgo(f.updated_at)}
                </span>
              </div>
              <div className="card-body">
                <CodeBlock code={f.spec.code} language={f.spec.runtime} className="fn-code" />
                <div className="form-row" style={{ alignItems: "flex-end" }}>
                  <label className="field" style={{ marginBottom: 0 }}>
                    <span>Params (JSON)</span>
                    <input
                      type="text"
                      className="mono"
                      placeholder='{"name": "cortex"}'
                      value={invokeParams[f.spec.name] ?? ""}
                      onChange={(e) =>
                        setInvokeParams((prev) => ({ ...prev, [f.spec.name]: e.target.value }))
                      }
                    />
                  </label>
                  <div style={{ flex: "0 0 auto", display: "flex", gap: 8 }}>
                    <button
                      className="btn primary"
                      disabled={busy === f.spec.name}
                      onClick={() => invoke(f.spec.name)}
                    >
                      {busy === f.spec.name ? "Running…" : "Invoke"}
                    </button>
                    <button className="btn danger" onClick={() => remove(f.spec.name)}>
                      Delete
                    </button>
                  </div>
                </div>
                {res && (
                  <div style={{ marginTop: 14 }}>
                    {res.ok ? (
                      <pre className="result-json">
                        {JSON.stringify(res.result, null, 2)}
                        {res.logs?.length ? `\n\n# logs\n${res.logs.join("\n")}` : ""}
                        {`\n\n# ${res.duration_ms}ms`}
                      </pre>
                    ) : (
                      <div className="error-banner">{res.error}</div>
                    )}
                  </div>
                )}
              </div>
            </div>
          );
        })
      )}
    </>
  );
}
