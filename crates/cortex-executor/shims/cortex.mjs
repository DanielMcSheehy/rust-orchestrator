// In-task Cortex platform bindings (JavaScript/TypeScript).
//
// Available to every task and function as the global `cortex` — the worker
// shim installs it before importing job code.
//
//   export async function handler(params, inputs) {
//     const rows = await cortex.query("SELECT sensor, AVG(value) v FROM readings GROUP BY sensor");
//     await cortex.ingest("aggregates", rows);
//     return cortex.invoke("notify", { count: rows.length });
//   }
const API = (process.env.CORTEX_API_URL ?? "http://127.0.0.1:7420").replace(/\/+$/, "");

async function request(path, body, contentType = "application/json") {
  const res = await fetch(API + path, {
    method: "POST",
    headers: { "content-type": contentType },
    body,
  });
  if (!res.ok) {
    throw new Error(`cortex api ${path}: HTTP ${res.status}: ${await res.text()}`);
  }
  return res.json();
}

export const cortex = {
  /** SQL over ingested datasets on the server's Rust (Polars) engine. */
  async query(sql, limit = 10_000) {
    const res = await request("/api/query", JSON.stringify({ sql, limit }));
    return res.rows;
  },
  /** Stream records into a dataset as NDJSON. */
  ingest(dataset, records) {
    const body = Array.from(records, (r) => JSON.stringify(r)).join("\n") + "\n";
    return request(`/api/ingest/${dataset}`, body, "application/x-ndjson");
  },
  /** Invoke a serverless function. */
  invoke(name, params = {}) {
    return request(`/api/functions/${name}/invoke`, JSON.stringify({ params }));
  },
};
