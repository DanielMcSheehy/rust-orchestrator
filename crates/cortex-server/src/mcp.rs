//! MCP (Model Context Protocol) server — makes Cortex a first-class tool
//! surface for AI agents.
//!
//! Implements the streamable-HTTP transport at `POST /mcp`: each request is
//! one JSON-RPC message, each response plain JSON (no SSE upgrade needed for
//! a stateless server). Supported methods: `initialize`, `ping`,
//! `tools/list`, `tools/call`, plus notification acknowledgement.
//!
//! Register it with e.g. `claude mcp add --transport http cortex
//! http://localhost:7420/mcp`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use cortex_core::{is_safe_name, Notebook, Runtime, WorkflowSpec};
use cortex_executor::ExecRequest;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::orchestrator::launch_run;
use crate::state::SharedState;

const PROTOCOL_VERSION: &str = "2025-06-18";

pub async fn handle(State(state): State<SharedState>, Json(msg): Json<Value>) -> Response {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Notifications (no id) are acknowledged with 202 and no body.
    if id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "cortex", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Cortex is a workflow orchestration platform. Define DAGs of \
                Python/TypeScript tasks, trigger runs, execute code directly, run SQL over \
                ingested datasets (embedded Polars) or external connectors, ingest NDJSON \
                data, and manage serverless functions."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => call_tool(&state, &params).await,
        other => Err(format!("method not found: {other}")),
    };

    let body = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(message) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": message }
        }),
    };
    Json(body).into_response()
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        }
    })
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool("cortex_stats", "Platform counters: workflows, runs, functions, datasets, ingested volume.", json!({}), &[]),
        tool(
            "execute_code",
            "Run a code snippet on the warm worker pool and get its result + logs. The code must define `handler(params, inputs)` (Python) or export it (JS/TS).",
            json!({
                "runtime": {"type": "string", "enum": ["python", "typescript", "javascript"]},
                "code": {"type": "string"},
                "params": {"type": "object"}
            }),
            &["runtime", "code"],
        ),
        tool(
            "query",
            "Run SQL. Default engine: embedded Polars over ingested datasets (dataset names are tables; dashes aliased to underscores; joins allowed). Pass `connector` to target a registered Postgres/ClickHouse/chDB source.",
            json!({
                "sql": {"type": "string"},
                "connector": {"type": "string"},
                "limit": {"type": "integer"}
            }),
            &["sql"],
        ),
        tool(
            "ingest",
            "Append records to a named dataset (creates it on first ingest). Workflows with an on_ingest trigger for the dataset run automatically.",
            json!({
                "dataset": {"type": "string"},
                "records": {"type": "array", "items": {"type": "object"}}
            }),
            &["dataset", "records"],
        ),
        tool("list_datasets", "List ingested datasets with record/byte counts.", json!({}), &[]),
        tool("list_workflows", "List workflows with their DAG specs.", json!({}), &[]),
        tool(
            "create_workflow",
            "Create or replace a workflow. `spec` is {name, params?, tasks: [{id, runtime, code, depends_on?, retries?, timeout_secs?}], triggers?: {every_secs?, on_ingest?}}. Task code defines `handler(params, inputs)`; `inputs` maps upstream task ids to their results.",
            json!({ "spec": {"type": "object"} }),
            &["spec"],
        ),
        tool(
            "trigger_workflow",
            "Start a run of a workflow by id or name. Returns the run; poll get_run for completion.",
            json!({
                "workflow": {"type": "string", "description": "workflow id or name"},
                "params": {"type": "object"}
            }),
            &["workflow"],
        ),
        tool(
            "get_run",
            "Get a run's state plus every task's state, result, error, and logs.",
            json!({ "run_id": {"type": "string"} }),
            &["run_id"],
        ),
        tool(
            "list_runs",
            "Recent runs, optionally filtered by workflow id.",
            json!({ "workflow_id": {"type": "string"}, "limit": {"type": "integer"} }),
            &[],
        ),
        tool(
            "invoke_function",
            "Invoke a deployed serverless function with params.",
            json!({ "name": {"type": "string"}, "params": {"type": "object"} }),
            &["name"],
        ),
        tool(
            "create_function",
            "Deploy (or update) a serverless function.",
            json!({
                "name": {"type": "string"},
                "runtime": {"type": "string", "enum": ["python", "typescript", "javascript"]},
                "code": {"type": "string"},
                "description": {"type": "string"}
            }),
            &["name", "runtime", "code"],
        ),
        tool(
            "create_notebook",
            "Create a notebook document with cells (markdown/code/sql) shown in the console.",
            json!({ "name": {"type": "string"}, "cells": {"type": "array"} }),
            &["name"],
        ),
    ]
}

async fn call_tool(state: &SharedState, params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().ok_or("missing tool name")?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match dispatch(state, name, args).await {
        Ok(value) => Ok(json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "isError": false,
        })),
        Err(message) => Ok(json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        })),
    }
}

async fn dispatch(state: &SharedState, tool: &str, args: Value) -> Result<Value, String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    match tool {
        "cortex_stats" => serde_json::to_value(state.store.stats().map_err(|e| err(&e))?)
            .map_err(|e| err(&e)),

        "execute_code" => {
            let runtime: Runtime =
                serde_json::from_value(args["runtime"].clone()).map_err(|e| err(&e))?;
            let code = args["code"].as_str().ok_or("missing code")?.to_string();
            let outcome = state
                .executor
                .execute(
                    ExecRequest {
                        runtime,
                        code,
                        params: args.get("params").cloned().unwrap_or(Value::Null),
                        inputs: Value::Null,
                        timeout_secs: 120,
                    },
                    None,
                )
                .await
                .map_err(|e| err(&e))?;
            Ok(json!({ "result": outcome.value, "logs": outcome.logs }))
        }

        "query" => {
            let sql = args["sql"].as_str().ok_or("missing sql")?;
            let limit = args["limit"].as_u64().unwrap_or(1000).clamp(1, 100_000) as usize;
            if let Some(connector_name) = args["connector"].as_str() {
                let connector = state
                    .store
                    .get_connector(connector_name)
                    .map_err(|e| err(&e))?;
                let res = crate::connectors::query(state, &connector, sql, limit).await?;
                return Ok(json!({ "rows": res.rows, "truncated": res.truncated }));
            }
            let datasets: Vec<String> = state
                .store
                .list_datasets()
                .map_err(|e| err(&e))?
                .into_iter()
                .map(|d| d.name)
                .collect();
            let dir = state.data_dir.join("datasets");
            let sql = sql.to_string();
            let outcome = tokio::task::spawn_blocking(move || {
                crate::data::run_query(&dir, &datasets, &sql, limit)
            })
            .await
            .map_err(|e| err(&e))??;
            let rows: Value = serde_json::from_slice(&outcome.rows_json).map_err(|e| err(&e))?;
            Ok(json!({ "rows": rows, "truncated": outcome.truncated }))
        }

        "ingest" => {
            let dataset = args["dataset"].as_str().ok_or("missing dataset")?;
            if !is_safe_name(dataset) {
                return Err("dataset name must match [a-zA-Z0-9_-]{1,64}".to_string());
            }
            let records = args["records"].as_array().ok_or("records must be an array")?;
            let dir = state.data_dir.join("datasets");
            tokio::fs::create_dir_all(&dir).await.map_err(|e| err(&e))?;
            let mut payload = String::new();
            for r in records {
                payload.push_str(&r.to_string());
                payload.push('\n');
            }
            let bytes = payload.len() as u64;
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("{dataset}.ndjson")))
                .await
                .map_err(|e| err(&e))?
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| err(&e))?;
            let ds = state
                .store
                .record_ingest(dataset, records.len() as u64, bytes)
                .map_err(|e| err(&e))?;
            serde_json::to_value(ds).map_err(|e| err(&e))
        }

        "list_datasets" => {
            serde_json::to_value(state.store.list_datasets().map_err(|e| err(&e))?)
                .map_err(|e| err(&e))
        }

        "list_workflows" => {
            serde_json::to_value(state.store.list_workflows().map_err(|e| err(&e))?)
                .map_err(|e| err(&e))
        }

        "create_workflow" => {
            let spec: WorkflowSpec =
                serde_json::from_value(args["spec"].clone()).map_err(|e| err(&e))?;
            cortex_core::validate_dag(&spec.tasks).map_err(|e| err(&e))?;
            // Upsert by name so agents can iterate on a workflow.
            let existing = state
                .store
                .list_workflows()
                .map_err(|e| err(&e))?
                .into_iter()
                .find(|w| w.spec.name == spec.name);
            let now = Utc::now();
            let wf = match existing {
                Some(mut wf) => {
                    wf.spec = spec;
                    wf.updated_at = now;
                    wf
                }
                None => cortex_core::Workflow {
                    id: Uuid::new_v4(),
                    spec,
                    created_at: now,
                    updated_at: now,
                },
            };
            state.store.put_workflow(&wf).map_err(|e| err(&e))?;
            serde_json::to_value(wf).map_err(|e| err(&e))
        }

        "trigger_workflow" => {
            let key = args["workflow"].as_str().ok_or("missing workflow")?;
            let workflow = match Uuid::parse_str(key) {
                Ok(id) => state.store.get_workflow(id).map_err(|e| err(&e))?,
                Err(_) => state
                    .store
                    .list_workflows()
                    .map_err(|e| err(&e))?
                    .into_iter()
                    .find(|w| w.spec.name == key)
                    .ok_or_else(|| format!("no workflow named `{key}`"))?,
            };
            let params = args.get("params").cloned().unwrap_or(Value::Null);
            let run = launch_run(state.clone(), workflow, params, "mcp").map_err(|e| err(&e))?;
            serde_json::to_value(run).map_err(|e| err(&e))
        }

        "get_run" => {
            let id: Uuid = args["run_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or("run_id must be a uuid")?;
            let run = state.store.get_run(id).map_err(|e| err(&e))?;
            let tasks = state.store.list_task_runs(id).map_err(|e| err(&e))?;
            Ok(json!({ "run": run, "tasks": tasks }))
        }

        "list_runs" => {
            let workflow_id = args["workflow_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok());
            let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 200) as u32;
            serde_json::to_value(
                state
                    .store
                    .list_runs(workflow_id, limit)
                    .map_err(|e| err(&e))?,
            )
            .map_err(|e| err(&e))
        }

        "invoke_function" => {
            let name = args["name"].as_str().ok_or("missing name")?;
            let mut func = state.store.get_function(name).map_err(|e| err(&e))?;
            let outcome = state
                .executor
                .execute(
                    ExecRequest {
                        runtime: func.spec.runtime,
                        code: func.spec.code.clone(),
                        params: args.get("params").cloned().unwrap_or(Value::Null),
                        inputs: Value::Null,
                        timeout_secs: func.spec.timeout_secs,
                    },
                    None,
                )
                .await
                .map_err(|e| err(&e))?;
            func.invocations += 1;
            let _ = state.store.put_function(&func);
            Ok(json!({ "result": outcome.value, "logs": outcome.logs }))
        }

        "create_function" => {
            if let Some(name) = args["name"].as_str() {
                if !is_safe_name(name) {
                    return Err("function name must match [a-zA-Z0-9_-]{1,64}".to_string());
                }
            }
            let spec: cortex_core::FunctionSpec = serde_json::from_value(json!({
                "name": args["name"],
                "runtime": args["runtime"],
                "code": args["code"],
                "description": args.get("description").cloned().unwrap_or(Value::Null),
            }))
            .map_err(|e| err(&e))?;
            let now = Utc::now();
            let func = match state.store.get_function(&spec.name) {
                Ok(mut f) => {
                    f.spec = spec;
                    f.updated_at = now;
                    f
                }
                Err(_) => cortex_core::Function {
                    id: Uuid::new_v4(),
                    spec,
                    invocations: 0,
                    created_at: now,
                    updated_at: now,
                },
            };
            state.store.put_function(&func).map_err(|e| err(&e))?;
            serde_json::to_value(func).map_err(|e| err(&e))
        }

        "create_notebook" => {
            let now = Utc::now();
            let nb = Notebook {
                id: Uuid::new_v4(),
                name: args["name"].as_str().ok_or("missing name")?.to_string(),
                cells: args.get("cells").cloned().unwrap_or(json!([])),
                created_at: now,
                updated_at: now,
            };
            state.store.put_notebook(&nb).map_err(|e| err(&e))?;
            serde_json::to_value(nb).map_err(|e| err(&e))
        }

        other => Err(format!("unknown tool: {other}")),
    }
}

use tokio::io::AsyncWriteExt;
