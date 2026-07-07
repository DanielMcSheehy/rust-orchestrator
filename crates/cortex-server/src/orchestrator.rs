//! The orchestrator drives one workflow run to completion: it walks the DAG
//! in topological layers, executes each layer's tasks in parallel (bounded by
//! `max_parallel_tasks`), feeds upstream results into downstream tasks, and
//! broadcasts every state change and log line onto the event stream.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use cortex_core::{topo_layers, CortexEvent, Run, RunState, TaskRun, TaskSpec, Workflow};
use cortex_executor::{ExecError, ExecRequest};
use serde_json::{Map, Value};
use tokio::sync::Semaphore;
use tracing::{error, info};

use crate::state::SharedState;

/// Create a pending run and its task-run rows, then hand it to the
/// background orchestrator. Returns the run immediately.
pub fn launch_run(
    state: SharedState,
    workflow: Workflow,
    params: Value,
    trigger: impl Into<String>,
) -> Result<Run, cortex_store::StoreError> {
    let effective = merge_params(&workflow.spec.params, &params);
    let run = Run::new(&workflow, effective, trigger);
    state.store.put_run(&run)?;
    for task in &workflow.spec.tasks {
        state.store.put_task_run(&TaskRun::new(run.id, task))?;
    }
    state.emit(CortexEvent::run_updated(run.clone()));

    let run_for_bg = run.clone();
    tokio::spawn(async move {
        execute_run(state, workflow, run_for_bg).await;
    });
    Ok(run)
}

async fn execute_run(state: SharedState, workflow: Workflow, mut run: Run) {
    info!(run = %run.id, workflow = %workflow.spec.name, "run started");
    run.state = RunState::Running;
    run.started_at = Some(Utc::now());
    persist_run(&state, &run);

    let tasks: HashMap<String, TaskSpec> = workflow
        .spec
        .tasks
        .iter()
        .map(|t| (t.id.clone(), t.clone()))
        .collect();
    let mut task_runs: HashMap<String, TaskRun> = state
        .store
        .list_task_runs(run.id)
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.task_id.clone(), t))
        .collect();

    let layers = topo_layers(&workflow.spec.tasks);
    let semaphore = Arc::new(Semaphore::new(workflow.spec.max_parallel_tasks.max(1)));
    let mut results: HashMap<String, Value> = HashMap::new();
    let mut failure: Option<String> = None;

    'layers: for layer in layers {
        let mut handles = Vec::new();
        for task_id in layer {
            let spec = tasks[&task_id].clone();
            let mut task_run = task_runs
                .remove(&task_id)
                .unwrap_or_else(|| TaskRun::new(run.id, &spec));
            let inputs = Value::Object(
                spec.depends_on
                    .iter()
                    .filter_map(|d| results.get(d).map(|v| (d.clone(), v.clone())))
                    .collect::<Map<String, Value>>(),
            );
            let params = merge_params(&run.params, &spec.params);
            let state = state.clone();
            let semaphore = semaphore.clone();
            let run_id = run.id;
            handles.push(tokio::spawn(async move {
                let _permit = semaphore.acquire().await.expect("semaphore open");
                let outcome = run_task(&state, run_id, &spec, &mut task_run, params, inputs).await;
                (task_run, outcome)
            }));
        }

        for handle in handles {
            match handle.await {
                Ok((task_run, Ok(value))) => {
                    results.insert(task_run.task_id.clone(), value);
                }
                Ok((task_run, Err(message))) => {
                    failure = Some(format!("task `{}` failed: {message}", task_run.task_id));
                }
                Err(join_err) => {
                    failure = Some(format!("task panicked: {join_err}"));
                }
            }
        }
        if failure.is_some() {
            break 'layers;
        }
    }

    // Tasks never reached (downstream of a failure) are marked cancelled.
    if failure.is_some() {
        for (_, mut tr) in task_runs.drain() {
            if !tr.state.is_terminal() {
                tr.state = RunState::Cancelled;
                tr.finished_at = Some(Utc::now());
                let _ = state.store.put_task_run(&tr);
                state.emit(CortexEvent::task_updated(tr));
            }
        }
    }

    run.finished_at = Some(Utc::now());
    match failure {
        Some(msg) => {
            error!(run = %run.id, "run failed: {msg}");
            run.state = RunState::Failed;
            run.error = Some(msg);
        }
        None => {
            info!(run = %run.id, "run completed");
            run.state = RunState::Completed;
        }
    }
    persist_run(&state, &run);
}

/// Execute one task with retries. Returns the task's result value, or the
/// final error message after all attempts are exhausted.
async fn run_task(
    state: &SharedState,
    run_id: uuid::Uuid,
    spec: &TaskSpec,
    task_run: &mut TaskRun,
    params: Value,
    inputs: Value,
) -> Result<Value, String> {
    task_run.state = RunState::Running;
    task_run.started_at = Some(Utc::now());
    persist_task(state, task_run);

    let max_attempts = spec.retries + 1;
    let mut last_error = String::new();
    for attempt in 1..=max_attempts {
        task_run.attempts = attempt;

        let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let forwarder_state = state.clone();
        let task_id = spec.id.clone();
        let forwarder = tokio::spawn(async move {
            let mut lines = Vec::new();
            while let Some(line) = log_rx.recv().await {
                forwarder_state.emit(CortexEvent::log(run_id, task_id.clone(), line.clone()));
                lines.push(line);
            }
            lines
        });

        let exec = state
            .executor
            .execute(
                ExecRequest {
                    runtime: spec.runtime,
                    code: spec.code.clone(),
                    params: params.clone(),
                    inputs: inputs.clone(),
                    timeout_secs: spec.timeout_secs,
                },
                Some(log_tx),
            )
            .await;
        if let Ok(lines) = forwarder.await {
            task_run.logs.extend(lines);
        }

        match exec {
            Ok(outcome) => {
                task_run.state = RunState::Completed;
                task_run.result = Some(outcome.value.clone());
                task_run.finished_at = Some(Utc::now());
                persist_task(state, task_run);
                return Ok(outcome.value);
            }
            Err(err) => {
                last_error = describe(&err);
                task_run.error = Some(last_error.clone());
                // Surface the workload's stack trace in the task logs so the
                // console shows *where* user code failed, not just the message.
                if let ExecError::Workload { trace, .. } = &err {
                    for line in trace.lines().filter(|l| !l.trim().is_empty()) {
                        task_run.logs.push(format!("[trace] {line}"));
                    }
                }
                if attempt < max_attempts {
                    state.emit(CortexEvent::log(
                        run_id,
                        spec.id.clone(),
                        format!("attempt {attempt}/{max_attempts} failed ({last_error}); retrying"),
                    ));
                    persist_task(state, task_run);
                }
            }
        }
    }

    task_run.state = RunState::Failed;
    task_run.finished_at = Some(Utc::now());
    persist_task(state, task_run);
    Err(last_error)
}

fn describe(err: &ExecError) -> String {
    match err {
        ExecError::Workload { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn persist_run(state: &SharedState, run: &Run) {
    if let Err(e) = state.store.put_run(run) {
        error!(run = %run.id, "failed to persist run: {e}");
    }
    state.emit(CortexEvent::run_updated(run.clone()));
}

fn persist_task(state: &SharedState, task: &TaskRun) {
    if let Err(e) = state.store.put_task_run(task) {
        error!(task = %task.id, "failed to persist task run: {e}");
    }
    state.emit(CortexEvent::task_updated(task.clone()));
}

/// Shallow-merge two JSON values. Objects merge key-by-key (`overlay` wins);
/// anything else: `overlay` replaces `base` unless it is null/absent.
pub fn merge_params(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                merged.insert(k.clone(), v.clone());
            }
            Value::Object(merged)
        }
        (b, Value::Null) => b.clone(),
        (_, o) => o.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::merge_params;
    use serde_json::json;

    #[test]
    fn overlay_wins_key_by_key() {
        let merged = merge_params(&json!({"a": 1, "b": 2}), &json!({"b": 3, "c": 4}));
        assert_eq!(merged, json!({"a": 1, "b": 3, "c": 4}));
    }

    #[test]
    fn null_overlay_keeps_base() {
        let merged = merge_params(&json!({"a": 1}), &json!(null));
        assert_eq!(merged, json!({"a": 1}));
    }
}
