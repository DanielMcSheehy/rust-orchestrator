//! HTTP surface: REST + SSE + streaming ingestion.

use std::convert::Infallible;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::Utc;
use cortex_core::{
    validate_dag, Connector, ConnectorKind, CortexEvent, Function, FunctionSpec, Notebook, Run,
    Runtime, Workflow, WorkflowSpec,
};
use cortex_executor::{ExecError, ExecRequest};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio_stream::wrappers::{BroadcastStream, UnboundedReceiverStream};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::orchestrator::{launch_run, merge_params};
use crate::state::SharedState;

pub fn api_router() -> Router<SharedState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/stats", get(stats))
        .route("/events", get(all_events))
        .route("/workflows", get(list_workflows).post(create_workflow))
        .route(
            "/workflows/{id}",
            get(get_workflow).put(update_workflow).delete(delete_workflow),
        )
        .route("/workflows/{id}/trigger", post(trigger_workflow))
        .route("/runs", get(list_runs))
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/events", get(run_events))
        .route("/functions", get(list_functions).post(create_function))
        .route(
            "/functions/{name}",
            get(get_function).delete(delete_function),
        )
        .route("/functions/{name}/invoke", post(invoke_function))
        .route(
            "/functions/{name}/invoke/stream",
            post(invoke_function_stream),
        )
        .route("/datasets", get(list_datasets))
        .route("/ingest/{dataset}", post(ingest))
        .route("/query", post(query))
        .route("/execute", post(execute))
        .route("/connectors", get(list_connectors).post(create_connector))
        .route("/connectors/{name}", delete(delete_connector))
        .route("/notebooks", get(list_notebooks).post(create_notebook))
        .route(
            "/notebooks/{id}",
            get(get_notebook).put(update_notebook).delete(delete_notebook),
        )
}

async fn healthz() -> Json<Value> {
    Json(json!({ "ok": true, "service": "cortex-server" }))
}

async fn stats(State(state): State<SharedState>) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(state.store.stats()?).unwrap()))
}

// ── workflows ────────────────────────────────────────────────────────────

async fn list_workflows(State(state): State<SharedState>) -> ApiResult<Json<Vec<Workflow>>> {
    Ok(Json(state.store.list_workflows()?))
}

async fn create_workflow(
    State(state): State<SharedState>,
    Json(spec): Json<WorkflowSpec>,
) -> ApiResult<(StatusCode, Json<Workflow>)> {
    validate_dag(&spec.tasks)?;
    let now = Utc::now();
    let wf = Workflow {
        id: Uuid::new_v4(),
        spec,
        created_at: now,
        updated_at: now,
    };
    state.store.put_workflow(&wf)?;
    Ok((StatusCode::CREATED, Json(wf)))
}

async fn get_workflow(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Workflow>> {
    Ok(Json(state.store.get_workflow(id)?))
}

async fn update_workflow(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(spec): Json<WorkflowSpec>,
) -> ApiResult<Json<Workflow>> {
    validate_dag(&spec.tasks)?;
    let mut wf = state.store.get_workflow(id)?;
    wf.spec = spec;
    wf.updated_at = Utc::now();
    state.store.put_workflow(&wf)?;
    Ok(Json(wf))
}

async fn delete_workflow(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    state.store.delete_workflow(id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct TriggerBody {
    #[serde(default)]
    params: Value,
}

async fn trigger_workflow(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    body: Option<Json<TriggerBody>>,
) -> ApiResult<(StatusCode, Json<Run>)> {
    let workflow = state.store.get_workflow(id)?;
    let params = body.map(|Json(b)| b.params).unwrap_or(Value::Null);
    let run = launch_run(state, workflow, params, "manual")?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

// ── runs ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RunsQuery {
    workflow_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    limit: u32,
}

fn default_limit() -> u32 {
    50
}

async fn list_runs(
    State(state): State<SharedState>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Json<Vec<Run>>> {
    Ok(Json(state.store.list_runs(q.workflow_id, q.limit.min(500))?))
}

async fn get_run(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let run = state.store.get_run(id)?;
    let mut tasks = state.store.list_task_runs(id)?;
    tasks.sort_by(|a, b| {
        a.started_at
            .unwrap_or(chrono::DateTime::<Utc>::MAX_UTC)
            .cmp(&b.started_at.unwrap_or(chrono::DateTime::<Utc>::MAX_UTC))
    });
    Ok(Json(json!({ "run": run, "tasks": tasks })))
}

// ── live event streams (SSE) ─────────────────────────────────────────────

fn sse_stream(
    state: SharedState,
    run_filter: Option<Uuid>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(move |item| {
        let event = match item {
            Ok(ev) => ev,
            // Slow consumer dropped some events; skip rather than kill the stream.
            Err(_) => return futures::future::ready(None),
        };
        if let Some(run_id) = run_filter {
            if event.run_id() != Some(run_id) {
                return futures::future::ready(None);
            }
        }
        let sse = Event::default()
            .json_data(&event)
            .expect("event serializes");
        futures::future::ready(Some(Ok::<_, Infallible>(sse)))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn all_events(State(state): State<SharedState>) -> impl IntoResponse {
    sse_stream(state, None)
}

async fn run_events(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    sse_stream(state, Some(id))
}

// ── serverless functions ─────────────────────────────────────────────────

async fn list_functions(State(state): State<SharedState>) -> ApiResult<Json<Vec<Function>>> {
    Ok(Json(state.store.list_functions()?))
}

async fn create_function(
    State(state): State<SharedState>,
    Json(spec): Json<FunctionSpec>,
) -> ApiResult<(StatusCode, Json<Function>)> {
    if spec.name.is_empty() || !is_safe_name(&spec.name) {
        return Err(ApiError::bad_request(
            "function name must match [a-zA-Z0-9_-]{1,64}",
        ));
    }
    let now = Utc::now();
    let func = match state.store.get_function(&spec.name) {
        Ok(mut existing) => {
            existing.spec = spec;
            existing.updated_at = now;
            existing
        }
        Err(_) => Function {
            id: Uuid::new_v4(),
            spec,
            invocations: 0,
            created_at: now,
            updated_at: now,
        },
    };
    state.store.put_function(&func)?;
    Ok((StatusCode::CREATED, Json(func)))
}

async fn get_function(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> ApiResult<Json<Function>> {
    Ok(Json(state.store.get_function(&name)?))
}

async fn delete_function(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    state.store.delete_function(&name)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct InvokeBody {
    #[serde(default)]
    params: Value,
}

async fn invoke_function(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    body: Option<Json<InvokeBody>>,
) -> ApiResult<Json<Value>> {
    let mut func = state.store.get_function(&name)?;
    let params = body.map(|Json(b)| b.params).unwrap_or(Value::Null);
    let started = std::time::Instant::now();
    let exec = state
        .executor
        .execute(
            ExecRequest {
                runtime: func.spec.runtime,
                code: func.spec.code.clone(),
                params: merge_params(&Value::Null, &params),
                inputs: Value::Null,
                timeout_secs: func.spec.timeout_secs,
            },
            None,
        )
        .await;
    let duration_ms = started.elapsed().as_millis() as u64;

    func.invocations += 1;
    state.store.put_function(&func)?;
    let ok = exec.is_ok();
    state.emit(CortexEvent::FunctionInvoked {
        ts: Utc::now(),
        name: name.clone(),
        ok,
        duration_ms,
    });

    match exec {
        Ok(outcome) => Ok(Json(json!({
            "ok": true,
            "result": outcome.value,
            "logs": outcome.logs,
            "duration_ms": duration_ms,
        }))),
        Err(err) => Ok(Json(json!({
            "ok": false,
            "error": err.to_string(),
            "duration_ms": duration_ms,
        }))),
    }
}

/// Streaming invocation: SSE events `log` (one per line, live) then a final
/// `result` or `error` event.
async fn invoke_function_stream(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    body: Option<Json<InvokeBody>>,
) -> ApiResult<impl IntoResponse> {
    let mut func = state.store.get_function(&name)?;
    let params = body.map(|Json(b)| b.params).unwrap_or(Value::Null);

    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let log_event_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(line) = log_rx.recv().await {
            let _ = log_event_tx.send(Event::default().event("log").data(line));
        }
    });

    let exec_state = state.clone();
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let exec = exec_state
            .executor
            .execute(
                ExecRequest {
                    runtime: func.spec.runtime,
                    code: func.spec.code.clone(),
                    params,
                    inputs: Value::Null,
                    timeout_secs: func.spec.timeout_secs,
                },
                Some(log_tx),
            )
            .await;
        let duration_ms = started.elapsed().as_millis() as u64;
        func.invocations += 1;
        let _ = exec_state.store.put_function(&func);
        let ok = exec.is_ok();
        exec_state.emit(CortexEvent::FunctionInvoked {
            ts: Utc::now(),
            name: func.spec.name.clone(),
            ok,
            duration_ms,
        });
        let final_event = match exec {
            Ok(outcome) => Event::default().event("result").data(
                json!({ "result": outcome.value, "duration_ms": duration_ms }).to_string(),
            ),
            Err(err) => Event::default()
                .event("error")
                .data(json!({ "error": err.to_string(), "duration_ms": duration_ms }).to_string()),
        };
        let _ = event_tx.send(final_event);
    });

    let stream = UnboundedReceiverStream::new(event_rx).map(Ok::<_, Infallible>);
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

// ── datasets & streaming ingestion ───────────────────────────────────────

async fn list_datasets(State(state): State<SharedState>) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(state.store.list_datasets()?).unwrap()))
}

#[derive(Deserialize)]
struct QueryBody {
    sql: String,
    #[serde(default = "default_query_limit")]
    limit: usize,
    /// Route to a named external connector instead of the embedded engine.
    #[serde(default)]
    connector: Option<String>,
}

fn default_query_limit() -> usize {
    10_000
}

/// SQL over ingested datasets — executed by the embedded Polars engine, or
/// routed to an external connector (Postgres / ClickHouse / chDB).
async fn query(
    State(state): State<SharedState>,
    Json(body): Json<QueryBody>,
) -> ApiResult<Json<Value>> {
    let limit = body.limit.clamp(1, 200_000);
    let started = std::time::Instant::now();

    if let Some(name) = &body.connector {
        let connector = state.store.get_connector(name)?;
        let result = crate::connectors::query(&state, &connector, &body.sql, limit)
            .await
            .map_err(ApiError::bad_request)?;
        return Ok(Json(json!({
            "rows": result.rows,
            "row_count": result.rows.len(),
            "truncated": result.truncated,
            "connector": name,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        })));
    }

    let datasets: Vec<String> = state
        .store
        .list_datasets()?
        .into_iter()
        .map(|d| d.name)
        .collect();
    let dir = state.data_dir.join("datasets");
    let outcome = tokio::task::spawn_blocking(move || {
        crate::data::run_query(&dir, &datasets, &body.sql, limit)
    })
    .await
    .map_err(|e| ApiError::internal(format!("query task failed: {e}")))?
    .map_err(ApiError::bad_request)?;

    let rows: Value = serde_json::from_slice(&outcome.rows_json)
        .map_err(|e| ApiError::internal(format!("bad result encoding: {e}")))?;
    Ok(Json(json!({
        "rows": rows,
        "row_count": outcome.row_count,
        "truncated": outcome.truncated,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    })))
}

// ── direct code execution ────────────────────────────────────────────────

#[derive(Deserialize)]
struct ExecuteBody {
    runtime: Runtime,
    code: String,
    #[serde(default)]
    params: Value,
    #[serde(default = "default_exec_timeout")]
    timeout_secs: u64,
}

fn default_exec_timeout() -> u64 {
    120
}

/// Run a snippet of Python/TypeScript/JavaScript on the warm worker pool and
/// return its result + logs. The code must define/export
/// `handler(params, inputs)` like any task.
async fn execute(
    State(state): State<SharedState>,
    Json(body): Json<ExecuteBody>,
) -> ApiResult<Json<Value>> {
    let started = std::time::Instant::now();
    let exec = state
        .executor
        .execute(
            ExecRequest {
                runtime: body.runtime,
                code: body.code,
                params: body.params,
                inputs: Value::Null,
                timeout_secs: body.timeout_secs.clamp(1, 600),
            },
            None,
        )
        .await;
    let duration_ms = started.elapsed().as_millis() as u64;
    match exec {
        Ok(outcome) => Ok(Json(json!({
            "ok": true,
            "result": outcome.value,
            "logs": outcome.logs,
            "duration_ms": duration_ms,
        }))),
        Err(ExecError::Workload { message, trace }) => Ok(Json(json!({
            "ok": false,
            "error": message,
            "trace": trace,
            "duration_ms": duration_ms,
        }))),
        Err(err) => Ok(Json(json!({
            "ok": false,
            "error": err.to_string(),
            "duration_ms": duration_ms,
        }))),
    }
}

// ── connectors ───────────────────────────────────────────────────────────

async fn list_connectors(State(state): State<SharedState>) -> ApiResult<Json<Vec<Connector>>> {
    Ok(Json(state.store.list_connectors()?))
}

#[derive(Deserialize)]
struct ConnectorBody {
    name: String,
    kind: ConnectorKind,
    #[serde(default)]
    url: String,
}

async fn create_connector(
    State(state): State<SharedState>,
    Json(body): Json<ConnectorBody>,
) -> ApiResult<(StatusCode, Json<Connector>)> {
    if !is_safe_name(&body.name) {
        return Err(ApiError::bad_request(
            "connector name must match [a-zA-Z0-9_-]{1,64}",
        ));
    }
    if matches!(body.kind, ConnectorKind::Postgres | ConnectorKind::Clickhouse)
        && body.url.is_empty()
    {
        return Err(ApiError::bad_request("this connector kind requires a url"));
    }
    let connector = Connector {
        name: body.name,
        kind: body.kind,
        url: body.url,
        created_at: Utc::now(),
    };
    state.store.put_connector(&connector)?;
    Ok((StatusCode::CREATED, Json(connector)))
}

async fn delete_connector(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    state.store.delete_connector(&name)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── notebooks ────────────────────────────────────────────────────────────

async fn list_notebooks(State(state): State<SharedState>) -> ApiResult<Json<Vec<Notebook>>> {
    Ok(Json(state.store.list_notebooks()?))
}

#[derive(Deserialize)]
struct NotebookBody {
    name: String,
    #[serde(default)]
    cells: Value,
}

async fn create_notebook(
    State(state): State<SharedState>,
    Json(body): Json<NotebookBody>,
) -> ApiResult<(StatusCode, Json<Notebook>)> {
    let now = Utc::now();
    let nb = Notebook {
        id: Uuid::new_v4(),
        name: body.name,
        cells: body.cells,
        created_at: now,
        updated_at: now,
    };
    state.store.put_notebook(&nb)?;
    Ok((StatusCode::CREATED, Json(nb)))
}

async fn get_notebook(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Notebook>> {
    Ok(Json(state.store.get_notebook(id)?))
}

async fn update_notebook(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(body): Json<NotebookBody>,
) -> ApiResult<Json<Notebook>> {
    let mut nb = state.store.get_notebook(id)?;
    nb.name = body.name;
    nb.cells = body.cells;
    nb.updated_at = Utc::now();
    state.store.put_notebook(&nb)?;
    Ok(Json(nb))
}

async fn delete_notebook(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    state.store.delete_notebook(id)?;
    Ok(StatusCode::NO_CONTENT)
}

fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Streaming NDJSON ingestion. The request body is consumed chunk-by-chunk
/// and appended to the dataset's NDJSON file — gigabyte payloads never sit
/// in memory. Workflows with an `on_ingest` trigger for this dataset are
/// launched once the payload is fully persisted.
async fn ingest(
    State(state): State<SharedState>,
    Path(dataset): Path<String>,
    body: Body,
) -> ApiResult<Json<Value>> {
    if !is_safe_name(&dataset) {
        return Err(ApiError::bad_request(
            "dataset name must match [a-zA-Z0-9_-]{1,64}",
        ));
    }

    let dir = state.data_dir.join("datasets");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{dataset}.ndjson"));
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;

    let mut records: u64 = 0;
    let mut bytes: u64 = 0;
    let mut ends_with_newline = true;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError::bad_request(format!("body stream error: {e}")))?;
        if chunk.is_empty() {
            continue;
        }
        records += chunk.iter().filter(|&&b| b == b'\n').count() as u64;
        bytes += chunk.len() as u64;
        ends_with_newline = chunk.last() == Some(&b'\n');
        file.write_all(&chunk).await?;
    }
    if bytes > 0 && !ends_with_newline {
        // Final record had no trailing newline: count it and terminate the line
        // so the next ingest starts fresh.
        records += 1;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;

    let ds = state.store.record_ingest(&dataset, records, bytes)?;
    state.emit(CortexEvent::Ingested {
        ts: Utc::now(),
        dataset: dataset.clone(),
        records,
        bytes,
    });

    // Fire ingest-triggered workflows.
    let mut triggered = Vec::new();
    for wf in state.store.list_workflows()? {
        if wf.spec.triggers.on_ingest.as_deref() == Some(dataset.as_str()) {
            let params = json!({
                "dataset": dataset,
                "records": records,
                "bytes": bytes,
                "path": path.to_string_lossy(),
            });
            let run = launch_run(state.clone(), wf, params, format!("ingest:{dataset}"))?;
            triggered.push(run.id);
        }
    }

    Ok(Json(json!({
        "dataset": ds,
        "ingested": { "records": records, "bytes": bytes },
        "triggered_runs": triggered,
    })))
}

#[cfg(test)]
mod tests {
    use super::is_safe_name;

    // Contract: dataset/function/connector names match `[a-zA-Z0-9_-]{1,64}`.
    #[test]
    fn accepts_alphanumeric_underscore_and_dash() {
        assert!(is_safe_name("a"));
        assert!(is_safe_name("Sensor-Readings_01"));
        assert!(is_safe_name("ABCxyz0123"));
    }

    #[test]
    fn rejects_empty_and_over_64_chars() {
        assert!(!is_safe_name(""));
        assert!(is_safe_name(&"a".repeat(64)));
        assert!(!is_safe_name(&"a".repeat(65)));
    }

    #[test]
    fn rejects_disallowed_characters() {
        for bad in ["has space", "dot.name", "slash/name", "café", "semi;colon", "quote\"x"] {
            assert!(!is_safe_name(bad), "should reject {bad:?}");
        }
    }
}
