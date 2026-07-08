//! Domain operations shared by the REST (`routes`) and MCP (`mcp`) surfaces.
//!
//! Both front-ends drive the same store + executor with identical semantics —
//! deploying a function, invoking it, running SQL, validating names. Keeping a
//! single implementation here is what stops the two surfaces from drifting
//! (e.g. one incrementing `invocations` and the other forgetting to).

use std::time::Instant;

use chrono::Utc;
use cortex_core::{Function, FunctionSpec};
use cortex_executor::{ExecError, ExecOutcome, ExecRequest};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;

/// True when `name` matches the platform-wide `[a-zA-Z0-9_-]{1,64}` rule used
/// for datasets, functions, and connectors.
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Reject a user-supplied name that isn't `[a-zA-Z0-9_-]{1,64}`. `kind` names
/// the thing being validated so the error reads e.g. "dataset name must ...".
pub fn require_safe_name(kind: &str, name: &str) -> ApiResult<()> {
    if is_safe_name(name) {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "{kind} name must match [a-zA-Z0-9_-]{{1,64}}"
        )))
    }
}

/// Deploy a serverless function, creating it or updating the existing one with
/// the same name (preserving its id, creation time, and invocation count).
pub fn upsert_function(state: &SharedState, spec: FunctionSpec) -> ApiResult<Function> {
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
    Ok(func)
}

/// The outcome of invoking a stored function: the execution result (which may
/// itself be a workload error), how long it took, and the resolved name.
pub struct Invocation {
    pub func_name: String,
    pub result: Result<ExecOutcome, ExecError>,
    pub duration_ms: u64,
}

/// Load a function by name, run it on the worker pool, and record the
/// invocation. `log_tx`, when provided, streams the worker's log lines live.
/// Emitting the `FunctionInvoked` event is left to the caller, since the two
/// surfaces differ on whether they broadcast it.
pub async fn invoke_function(
    state: &SharedState,
    name: &str,
    params: Value,
    log_tx: Option<UnboundedSender<String>>,
) -> ApiResult<Invocation> {
    let mut func = state.store.get_function(name)?;
    let started = Instant::now();
    let result = state
        .executor
        .execute(
            ExecRequest {
                runtime: func.spec.runtime,
                code: func.spec.code.clone(),
                params,
                inputs: Value::Null,
                timeout_secs: func.spec.timeout_secs,
            },
            log_tx,
        )
        .await;
    let duration_ms = started.elapsed().as_millis() as u64;
    func.invocations += 1;
    let _ = state.store.put_function(&func);
    Ok(Invocation {
        func_name: func.spec.name,
        result,
        duration_ms,
    })
}

/// Rows returned by [`run_sql`], plus enough metadata for callers to build
/// their own response envelope.
pub struct SqlResult {
    pub rows: Value,
    pub row_count: usize,
    pub truncated: bool,
    /// The connector the query was routed to, or `None` for the embedded engine.
    pub connector: Option<String>,
}

/// Run a SQL query, either against a registered external `connector` or the
/// embedded Polars engine over ingested datasets. Callers pre-clamp `limit`.
pub async fn run_sql(
    state: &SharedState,
    sql: &str,
    limit: usize,
    connector: Option<&str>,
) -> ApiResult<SqlResult> {
    if let Some(name) = connector {
        let connector = state.store.get_connector(name)?;
        let result = crate::connectors::query(state, &connector, sql, limit)
            .await
            .map_err(ApiError::bad_request)?;
        return Ok(SqlResult {
            row_count: result.rows.len(),
            rows: Value::Array(result.rows),
            truncated: result.truncated,
            connector: Some(name.to_string()),
        });
    }

    let datasets: Vec<String> = state
        .store
        .list_datasets()?
        .into_iter()
        .map(|d| d.name)
        .collect();
    let dir = state.data_dir.join("datasets");
    let sql = sql.to_string();
    let outcome = tokio::task::spawn_blocking(move || {
        crate::data::run_query(&dir, &datasets, &sql, limit)
    })
    .await
    .map_err(|e| ApiError::internal(format!("query task failed: {e}")))?
    .map_err(ApiError::bad_request)?;

    let rows: Value = serde_json::from_slice(&outcome.rows_json)
        .map_err(|e| ApiError::internal(format!("bad result encoding: {e}")))?;
    Ok(SqlResult {
        rows,
        row_count: outcome.row_count,
        truncated: outcome.truncated,
        connector: None,
    })
}
