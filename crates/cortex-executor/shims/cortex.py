"""In-task Cortex platform bindings (Python).

Available to every task and function as `import cortex` — the worker shim
puts this module on `sys.path`. Local API calls deliberately bypass any
configured HTTP proxy.

    import cortex

    def handler(params, inputs):
        rows = cortex.query("SELECT sensor, AVG(value) v FROM readings GROUP BY sensor")
        cortex.ingest("aggregates", rows)
        return cortex.invoke("notify", {"count": len(rows)})
"""
import json
import os
import urllib.request

_API = os.environ.get("CORTEX_API_URL", "http://127.0.0.1:7420").rstrip("/")
_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def _request(method, path, data, content_type="application/json"):
    req = urllib.request.Request(
        _API + path, data=data, method=method, headers={"content-type": content_type}
    )
    with _OPENER.open(req) as resp:
        return json.loads(resp.read())


def query(sql, limit=10_000):
    """Run SQL against ingested datasets on the server's Rust (Polars)
    dataframe engine; returns a list of row dicts."""
    res = _request("POST", "/api/query", json.dumps({"sql": sql, "limit": limit}).encode())
    return res["rows"]


def ingest(dataset, records):
    """Stream an iterable of JSON-serializable records into a dataset."""
    payload = b"".join(
        json.dumps(r, separators=(",", ":")).encode() + b"\n" for r in records
    )
    return _request("POST", f"/api/ingest/{dataset}", payload, "application/x-ndjson")


def invoke(name, params=None):
    """Invoke a serverless function and return its response."""
    return _request(
        "POST",
        f"/api/functions/{name}/invoke",
        json.dumps({"params": params or {}}).encode(),
    )
