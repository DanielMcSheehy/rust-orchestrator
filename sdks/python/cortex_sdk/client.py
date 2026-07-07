"""HTTP client for the Cortex server. Standard library only."""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from typing import Any, Iterable, Iterator, Optional, Union

from .flow import Flow

TERMINAL_STATES = {"completed", "failed", "cancelled"}


class CortexError(RuntimeError):
    def __init__(self, status: int, message: str):
        super().__init__(f"HTTP {status}: {message}")
        self.status = status


class CortexClient:
    def __init__(self, base_url: str = "http://localhost:7420", timeout: float = 30.0):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    # ── plumbing ─────────────────────────────────────────────────────────

    def _request(
        self,
        method: str,
        path: str,
        body: Optional[Union[bytes, dict]] = None,
        stream: bool = False,
    ):
        data = None
        headers = {"accept": "application/json"}
        if isinstance(body, dict):
            data = json.dumps(body).encode()
            headers["content-type"] = "application/json"
        elif isinstance(body, bytes):
            data = body
            headers["content-type"] = "application/x-ndjson"
        req = urllib.request.Request(
            f"{self.base_url}{path}", data=data, method=method, headers=headers
        )
        try:
            resp = urllib.request.urlopen(req, timeout=None if stream else self.timeout)
        except urllib.error.HTTPError as e:
            detail = e.read().decode(errors="replace")
            try:
                detail = json.loads(detail).get("error", detail)
            except (ValueError, AttributeError):
                pass
            raise CortexError(e.code, detail) from None
        if stream:
            return resp
        raw = resp.read()
        return json.loads(raw) if raw else None

    # ── workflows & runs ─────────────────────────────────────────────────

    def deploy(self, flow: Flow) -> dict[str, Any]:
        """Create the flow on the server (or update it if the name exists)."""
        existing = {w["spec"]["name"]: w for w in self.list_workflows()}
        if flow.name in existing:
            wf_id = existing[flow.name]["id"]
            return self._request("PUT", f"/api/workflows/{wf_id}", flow.spec())
        return self._request("POST", "/api/workflows", flow.spec())

    def list_workflows(self) -> list[dict[str, Any]]:
        return self._request("GET", "/api/workflows")

    def get_workflow(self, workflow_id: str) -> dict[str, Any]:
        return self._request("GET", f"/api/workflows/{workflow_id}")

    def delete_workflow(self, workflow_id: str) -> None:
        self._request("DELETE", f"/api/workflows/{workflow_id}")

    def trigger(
        self,
        workflow_id: str,
        params: Optional[dict[str, Any]] = None,
        wait: bool = False,
        poll_interval: float = 0.5,
        timeout: float = 3600.0,
    ) -> dict[str, Any]:
        run = self._request(
            "POST", f"/api/workflows/{workflow_id}/trigger", {"params": params or {}}
        )
        if not wait:
            return run
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            current = self.get_run(run["id"])["run"]
            if current["state"] in TERMINAL_STATES:
                return current
            time.sleep(poll_interval)
        raise TimeoutError(f"run {run['id']} did not finish within {timeout}s")

    def list_runs(
        self, workflow_id: Optional[str] = None, limit: int = 50
    ) -> list[dict[str, Any]]:
        query = f"?limit={limit}"
        if workflow_id:
            query += f"&workflow_id={workflow_id}"
        return self._request("GET", f"/api/runs{query}")

    def get_run(self, run_id: str) -> dict[str, Any]:
        """Returns ``{"run": ..., "tasks": [...]}``."""
        return self._request("GET", f"/api/runs/{run_id}")

    # ── serverless functions ─────────────────────────────────────────────

    def create_function(
        self,
        name: str,
        code: str,
        runtime: str = "python",
        description: Optional[str] = None,
        timeout_secs: int = 300,
    ) -> dict[str, Any]:
        return self._request(
            "POST",
            "/api/functions",
            {
                "name": name,
                "code": code,
                "runtime": runtime,
                "description": description,
                "timeout_secs": timeout_secs,
            },
        )

    def list_functions(self) -> list[dict[str, Any]]:
        return self._request("GET", "/api/functions")

    def invoke(self, name: str, params: Optional[dict[str, Any]] = None) -> dict[str, Any]:
        return self._request(
            "POST", f"/api/functions/{name}/invoke", {"params": params or {}}
        )

    def delete_function(self, name: str) -> None:
        self._request("DELETE", f"/api/functions/{name}")

    # ── data ingestion ───────────────────────────────────────────────────

    def ingest(self, dataset: str, records: Iterable[Any]) -> dict[str, Any]:
        """Stream an iterable of JSON-serializable records as NDJSON."""
        payload = b"".join(
            json.dumps(r, separators=(",", ":")).encode() + b"\n" for r in records
        )
        return self._request("POST", f"/api/ingest/{dataset}", payload)

    def list_datasets(self) -> list[dict[str, Any]]:
        return self._request("GET", "/api/datasets")

    def query(
        self, sql: str, limit: int = 10_000, connector: Optional[str] = None
    ) -> list[dict[str, Any]]:
        """Run SQL against ingested datasets on the server's Rust (Polars)
        dataframe engine, or against a registered external connector
        (Postgres / ClickHouse / chDB). Returns row dicts."""
        body: dict[str, Any] = {"sql": sql, "limit": limit}
        if connector:
            body["connector"] = connector
        return self._request("POST", "/api/query", body)["rows"]

    def execute(
        self,
        code: str,
        runtime: str = "python",
        params: Optional[dict[str, Any]] = None,
        timeout_secs: int = 120,
    ) -> dict[str, Any]:
        """Run a code snippet on the server's warm worker pool. The code must
        define `handler(params, inputs)`. Returns
        ``{"ok", "result", "logs", "duration_ms"}`` (or ``"error"``/"trace")."""
        return self._request(
            "POST",
            "/api/execute",
            {"runtime": runtime, "code": code, "params": params or {}, "timeout_secs": timeout_secs},
        )

    # ── live events ──────────────────────────────────────────────────────

    def events(self, run_id: Optional[str] = None) -> Iterator[dict[str, Any]]:
        """Yield live server events (SSE). Blocks; iterate in a thread if needed."""
        path = f"/api/runs/{run_id}/events" if run_id else "/api/events"
        resp = self._request("GET", path, stream=True)
        yield from self._parse_sse(resp)

    @staticmethod
    def _parse_sse(resp) -> Iterator[dict[str, Any]]:
        for raw in resp:
            line = raw.decode(errors="replace").strip()
            if line.startswith("data:"):
                yield json.loads(line[5:].strip())

    def stream_run(self, run_id: str) -> Iterator[dict[str, Any]]:
        """Yield events for one run until it reaches a terminal state.

        Attaches to the event stream *before* checking the run's current
        state, so a run that finished in the meantime returns immediately
        instead of blocking on events that already happened.
        """
        resp = self._request("GET", f"/api/runs/{run_id}/events", stream=True)
        try:
            if self.get_run(run_id)["run"]["state"] in TERMINAL_STATES:
                return
            for event in self._parse_sse(resp):
                yield event
                if (
                    event.get("type") == "run_updated"
                    and event["run"]["state"] in TERMINAL_STATES
                ):
                    return
        finally:
            resp.close()

    def stats(self) -> dict[str, Any]:
        return self._request("GET", "/api/stats")
