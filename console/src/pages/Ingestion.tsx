import { useCallback, useEffect, useState } from "react";
import { api, formatBytes, timeAgo, useEvents } from "../api";
import CodeEditor from "../components/CodeEditor";
import ResultView from "../components/ResultView";
import { Empty } from "../components/ui";
import type { Connector, ConnectorKind, Dataset } from "../types";

interface IngestResponse {
  ingested: { records: number; bytes: number };
  triggered_runs: string[];
}

interface QueryResponse {
  rows: Array<Record<string, unknown>>;
  row_count: number;
  truncated: boolean;
  elapsed_ms: number;
}

function QueryPanel({ datasets, connectors }: { datasets: Dataset[]; connectors: Connector[] }) {
  const first = datasets[0]?.name.replace(/-/g, "_") ?? "my_dataset";
  const [sql, setSql] = useState(`SELECT * FROM ${first} LIMIT 20`);
  const [connector, setConnector] = useState("");
  const [result, setResult] = useState<QueryResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(
        await api.post<QueryResponse>("/api/query", {
          sql,
          limit: 1000,
          connector: connector || undefined,
        }),
      );
    } catch (e) {
      setResult(null);
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="card">
      <div className="card-head">
        <h2>Query</h2>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <select value={connector} onChange={(e) => setConnector(e.target.value)} style={{ width: "auto" }}>
            <option value="">datasets · polars (rust)</option>
            {connectors.map((c) => (
              <option key={c.name} value={c.name}>
                {c.name} · {c.kind}
              </option>
            ))}
          </select>
          <button className="btn primary sm" disabled={busy} onClick={run}>
            {busy ? "Running…" : "Run query"}
          </button>
        </div>
      </div>
      <div className="card-body">
        <CodeEditor value={sql} language="sql" minRows={3} onChange={setSql} onRun={run} />
        {error && (
          <div className="error-banner" style={{ marginTop: 12, marginBottom: 0 }}>
            {error}
          </div>
        )}
        {result && (
          <p className="muted" style={{ margin: "12px 0 8px", fontSize: 12 }}>
            {result.row_count.toLocaleString()} row{result.row_count === 1 ? "" : "s"}
            {result.truncated ? " (truncated)" : ""} · {result.elapsed_ms}ms
          </p>
        )}
        {result && result.rows.length > 0 && <ResultView value={result.rows} />}
      </div>
    </div>
  );
}

function ConnectorsPanel({
  connectors,
  onChange,
}: {
  connectors: Connector[];
  onChange: () => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ConnectorKind>("postgres");
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);

  const add = async () => {
    setError(null);
    try {
      await api.post("/api/connectors", { name, kind, url });
      setName("");
      setUrl("");
      onChange();
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const remove = async (n: string) => {
    await api.delete(`/api/connectors/${n}`);
    onChange();
  };

  return (
    <div className="card">
      <div className="card-head">
        <h2>Connectors</h2>
        <span className="muted" style={{ fontSize: 12 }}>
          query external Postgres / ClickHouse / chDB through the same API
        </span>
      </div>
      <div className="card-body">
        {error && <div className="error-banner">{error}</div>}
        <div className="form-row" style={{ alignItems: "flex-end" }}>
          <label className="field" style={{ marginBottom: 0, flex: "0 0 180px" }}>
            <span>Name</span>
            <input type="text" value={name} placeholder="warehouse" onChange={(e) => setName(e.target.value)} />
          </label>
          <label className="field" style={{ marginBottom: 0, flex: "0 0 140px" }}>
            <span>Kind</span>
            <select value={kind} onChange={(e) => setKind(e.target.value as ConnectorKind)}>
              <option value="postgres">postgres</option>
              <option value="clickhouse">clickhouse</option>
              <option value="chdb">chdb (embedded)</option>
            </select>
          </label>
          <label className="field" style={{ marginBottom: 0 }}>
            <span>URL</span>
            <input
              type="text"
              className="mono"
              disabled={kind === "chdb"}
              placeholder={
                kind === "postgres"
                  ? "postgres://user:pass@host:5432/db"
                  : kind === "clickhouse"
                    ? "http://host:8123/?user=default"
                    : "runs in the Python worker (pip install chdb)"
              }
              value={kind === "chdb" ? "" : url}
              onChange={(e) => setUrl(e.target.value)}
            />
          </label>
          <button className="btn primary" style={{ flex: "0 0 auto" }} disabled={!name} onClick={add}>
            Add
          </button>
        </div>
        {connectors.length > 0 && (
          <div style={{ marginTop: 14, display: "flex", gap: 8, flexWrap: "wrap" }}>
            {connectors.map((c) => (
              <span key={c.name} className="connector-chip">
                {c.name} <span className="muted">· {c.kind}</span>
                <button onClick={() => remove(c.name)} title="Remove">✕</button>
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export default function Ingestion() {
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [connectors, setConnectors] = useState<Connector[]>([]);
  const [name, setName] = useState("sensor-readings");
  const [payload, setPayload] = useState('{"sensor": "a", "value": 0.72}\n{"sensor": "b", "value": 0.41}');
  const [file, setFile] = useState<File | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.get<Dataset[]>("/api/datasets").then(setDatasets).catch(() => {});
    api.get<Connector[]>("/api/connectors").then(setConnectors).catch(() => {});
  }, []);

  useEffect(refresh, [refresh]);
  useEvents((ev) => {
    if (ev.type === "ingested") refresh();
  });

  const ingest = async () => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const body = file ? await file.text() : payload;
      const res = await api.post<IngestResponse>(`/api/ingest/${name}`, body);
      setNotice(
        `Ingested ${res.ingested.records.toLocaleString()} records (${formatBytes(res.ingested.bytes)})` +
          (res.triggered_runs.length ? ` — triggered ${res.triggered_runs.length} run(s)` : ""),
      );
      setFile(null);
      refresh();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="page-head">
        <div>
          <h1>Data</h1>
          <p>
            Stream NDJSON into named datasets, then query them with SQL — workflows with an{" "}
            <code>on_ingest</code> trigger run automatically after each batch.
          </p>
        </div>
      </div>

      {(datasets.length > 0 || connectors.length > 0) && (
        <QueryPanel datasets={datasets} connectors={connectors} />
      )}

      <div className="card">
        <div className="card-head">
          <h2>Ingest data</h2>
          <button className="btn primary sm" disabled={busy || !name} onClick={ingest}>
            {busy ? "Streaming…" : "Ingest"}
          </button>
        </div>
        <div className="card-body">
          {error && <div className="error-banner">{error}</div>}
          {notice && (
            <div
              className="error-banner"
              style={{
                background: "var(--good-soft)",
                borderColor: "rgba(12,163,12,0.4)",
                color: "#4ade4a",
              }}
            >
              {notice}
            </div>
          )}
          <div className="form-row">
            <label className="field">
              <span>Dataset</span>
              <input type="text" value={name} onChange={(e) => setName(e.target.value)} />
            </label>
            <label className="field">
              <span>Or upload an NDJSON file</span>
              <input
                type="file"
                accept=".ndjson,.jsonl,.json,.txt"
                onChange={(e) => setFile(e.target.files?.[0] ?? null)}
              />
            </label>
          </div>
          {!file && (
            <label className="field">
              <span>Records (one JSON object per line)</span>
              <CodeEditor value={payload} language="json" minRows={6} onChange={setPayload} />
            </label>
          )}
          {file && (
            <p className="muted">
              Will stream <strong>{file.name}</strong> ({formatBytes(file.size)}) to{" "}
              <code>{name}</code>.
            </p>
          )}
        </div>
      </div>

      <div className="card">
        <div className="card-head">
          <h2>Datasets</h2>
        </div>
        {datasets.length === 0 ? (
          <Empty title="Nothing ingested yet" hint="Stream a batch above or POST to /api/ingest/{dataset}." />
        ) : (
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th className="num">Records</th>
                <th className="num">Size</th>
                <th className="num">Last ingest</th>
              </tr>
            </thead>
            <tbody>
              {datasets.map((d) => (
                <tr key={d.name}>
                  <td style={{ color: "var(--ink)", fontWeight: 550 }}>{d.name}</td>
                  <td className="num">{d.records.toLocaleString()}</td>
                  <td className="num">{formatBytes(d.bytes)}</td>
                  <td className="num muted">{timeAgo(d.updated_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <ConnectorsPanel connectors={connectors} onChange={refresh} />
    </>
  );
}
