//! cortex-executor — runs Python and TypeScript/JavaScript workloads in
//! isolated OS processes.
//!
//! Every execution:
//!   1. writes the job code to a scratch dir (`job.py` / `job.ts` / `job.mjs`),
//!   2. spawns the matching worker shim (embedded in this crate) with
//!      stdin/stdout piped,
//!   3. sends one JSON request line, then streams JSON-lines events back —
//!      logs are forwarded live through an mpsc channel, the final line is
//!      the result or an error.
//!
//! Timeouts kill the whole process, so runaway user code can't wedge the
//! orchestrator.
//!
//! Workers can run at three isolation tiers — direct processes (default),
//! containers, or microVMs via a VM-backed OCI runtime — selected with
//! `CORTEX_ISOLATION`; see [`isolation`].

//! In `process` mode, finished interpreters are kept warm and reused for
//! subsequent jobs (see [`pool`]), which cuts per-task overhead from tens of
//! milliseconds to roughly one.

pub mod isolation;
mod pool;

use std::process::Stdio;
use std::time::{Duration, Instant};

use cortex_core::Runtime;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

pub use isolation::{ContainerConfig, Isolation};
use isolation::LaunchPlan;
use pool::{PoolKind, WorkerPool};

const PYTHON_SHIM: &str = include_str!("../shims/worker.py");
const NODE_SHIM: &str = include_str!("../shims/worker.mjs");
const PYTHON_BINDINGS: &str = include_str!("../shims/cortex.py");
const NODE_BINDINGS: &str = include_str!("../shims/cortex.mjs");

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("executor init failed: {0}")]
    Init(String),
    #[error("failed to spawn worker: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("workload failed: {message}")]
    Workload { message: String, trace: String },
    #[error("workload timed out after {0}s")]
    Timeout(u64),
    #[error("worker exited without producing a result")]
    NoResult,
    #[error("malformed worker event: {0}")]
    Protocol(String),
}

/// A single unit of work to execute.
#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub runtime: Runtime,
    pub code: String,
    /// Free-form parameters, passed as the handler's first argument.
    pub params: Value,
    /// Upstream task results keyed by task id, passed as the second argument.
    pub inputs: Value,
    pub timeout_secs: u64,
}

#[derive(Debug)]
pub struct ExecOutcome {
    pub value: Value,
    pub logs: Vec<String>,
    pub duration: Duration,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WorkerEvent {
    Log { line: String },
    Result { value: Value },
    Error { message: String, #[serde(default)] trace: String },
}

/// Spawns worker processes. Cheap to clone via `Arc`; keeps the shim files
/// materialized in a temp dir for its lifetime.
pub struct Executor {
    shim_dir: tempfile::TempDir,
    isolation: Isolation,
    /// Warm interpreter pool — `process` isolation only; container/microVM
    /// modes get a fresh sandbox per task by design.
    pool: Option<WorkerPool>,
}

impl Executor {
    /// Isolation mode is read from the environment (`CORTEX_ISOLATION` and
    /// friends); defaults to plain process execution.
    pub fn new() -> Result<Self, ExecError> {
        let isolation = Isolation::from_env().map_err(|e| ExecError::Init(e.to_string()))?;
        Self::with_isolation(isolation)
    }

    pub fn with_isolation(isolation: Isolation) -> Result<Self, ExecError> {
        let shim_dir = tempfile::Builder::new().prefix("cortex-shims-").tempdir()?;
        std::fs::write(shim_dir.path().join("worker.py"), PYTHON_SHIM)?;
        std::fs::write(shim_dir.path().join("worker.mjs"), NODE_SHIM)?;
        std::fs::write(shim_dir.path().join("cortex.py"), PYTHON_BINDINGS)?;
        std::fs::write(shim_dir.path().join("cortex.mjs"), NODE_BINDINGS)?;
        let pool_enabled = std::env::var("CORTEX_WORKER_POOL")
            .map(|v| v != "0")
            .unwrap_or(true);
        let pool = match &isolation {
            Isolation::Process { python_bin, node_bin } if pool_enabled => Some(
                WorkerPool::new(python_bin.clone(), node_bin.clone(), shim_dir.path()),
            ),
            _ => None,
        };
        tracing::info!(
            mode = isolation.mode_name(),
            warm_pool = pool.is_some(),
            "executor configured"
        );
        Ok(Executor {
            shim_dir,
            isolation,
            pool,
        })
    }

    pub fn isolation_mode(&self) -> &'static str {
        self.isolation.mode_name()
    }

    /// Execute a workload, streaming each log line into `log_tx` as it is
    /// produced by the worker process.
    pub async fn execute(
        &self,
        req: ExecRequest,
        log_tx: Option<UnboundedSender<String>>,
    ) -> Result<ExecOutcome, ExecError> {
        if let Some(pool) = &self.pool {
            return self.execute_pooled(pool, req, log_tx).await;
        }
        self.execute_oneshot(req, log_tx).await
    }

    /// Run a job on a warm pooled interpreter (`process` isolation).
    async fn execute_pooled(
        &self,
        pool: &WorkerPool,
        req: ExecRequest,
        log_tx: Option<UnboundedSender<String>>,
    ) -> Result<ExecOutcome, ExecError> {
        let started = Instant::now();
        let (job_dir, job_file) = write_job_dir(&req)?;
        let request_line = serde_json::to_string(&json!({
            "entry": job_dir.path().join(job_file),
            "params": req.params,
            "inputs": req.inputs,
        }))
        .expect("request serializes") + "\n";

        let kind = match req.runtime {
            Runtime::Python => PoolKind::Python,
            Runtime::Typescript | Runtime::Javascript => PoolKind::Node,
        };
        let mut worker = pool.checkout(kind)?;
        // An idle worker may have died while parked; if the handoff write
        // fails the job hasn't started, so retry once on a fresh spawn.
        if worker.stdin.write_all(request_line.as_bytes()).await.is_err() {
            worker = pool.spawn_fresh(kind)?;
            worker.stdin.write_all(request_line.as_bytes()).await?;
        }

        let timeout = Duration::from_secs(req.timeout_secs.max(1));
        let mut logs = Vec::new();
        let result = {
            let read_job = async {
                loop {
                    let Some(line) = worker.lines.next_line().await? else {
                        return Err(ExecError::NoResult); // EOF: worker died mid-job
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match parse_event(&line)? {
                        WorkerEvent::Log { line } => {
                            if let Some(tx) = &log_tx {
                                let _ = tx.send(line.clone());
                            }
                            logs.push(line);
                        }
                        WorkerEvent::Result { value } => return Ok(value),
                        WorkerEvent::Error { message, trace } => {
                            return Err(ExecError::Workload { message, trace })
                        }
                    }
                }
            };
            tokio::time::timeout(timeout, read_job).await
        };

        match result {
            // Timed out: the worker is wedged — dropping it kills the process.
            Err(_) => Err(ExecError::Timeout(req.timeout_secs)),
            Ok(Ok(value)) => {
                pool.checkin(worker);
                Ok(ExecOutcome {
                    value,
                    logs,
                    duration: started.elapsed(),
                })
            }
            // The job failed but the worker finished the protocol cleanly —
            // it is still healthy and reusable.
            Ok(Err(err @ ExecError::Workload { .. })) => {
                pool.checkin(worker);
                Err(err)
            }
            // Protocol violation or dead worker: don't reuse.
            Ok(Err(err)) => Err(err),
        }
    }

    /// Spawn a dedicated worker for one job (container/microVM isolation, or
    /// `process` mode with the pool disabled).
    async fn execute_oneshot(
        &self,
        req: ExecRequest,
        log_tx: Option<UnboundedSender<String>>,
    ) -> Result<ExecOutcome, ExecError> {
        let started = Instant::now();
        let (job_dir, job_file) = write_job_dir(&req)?;

        // The temp dir's unique suffix doubles as the container-name nonce.
        let nonce = job_dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("cortex-job-"))
            .unwrap_or("0")
            .to_string();
        let plan = isolation::plan(
            &self.isolation,
            req.runtime,
            self.shim_dir.path(),
            job_dir.path(),
            job_file,
            &nonce,
        );

        let mut cmd = Command::new(&plan.program);
        cmd.args(&plan.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let request_line = serde_json::to_string(&json!({
            "entry": plan.entry,
            "params": req.params,
            "inputs": req.inputs,
        }))
        .expect("request serializes") + "\n";

        let mut stdin = child.stdin.take().expect("stdin piped");
        stdin.write_all(request_line.as_bytes()).await?;
        drop(stdin);

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let timeout = Duration::from_secs(req.timeout_secs.max(1));
        let work = self.drive(stdout, stderr, log_tx);
        let outcome = match tokio::time::timeout(timeout, work).await {
            Ok(outcome) => outcome,
            Err(_) => {
                let _ = child.kill().await;
                kill_container(&plan).await;
                return Err(ExecError::Timeout(req.timeout_secs));
            }
        };
        // Reap the child so it doesn't linger as a zombie.
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

        let (value, logs) = outcome?;
        Ok(ExecOutcome {
            value,
            logs,
            duration: started.elapsed(),
        })
    }

    async fn drive(
        &self,
        stdout: tokio::process::ChildStdout,
        stderr: tokio::process::ChildStderr,
        log_tx: Option<UnboundedSender<String>>,
    ) -> Result<(Value, Vec<String>), ExecError> {
        // stderr (interpreter noise, stack traces printed by the runtime)
        // becomes log lines too, so users see everything in one stream.
        let stderr_tx = log_tx.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut collected = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let tagged = format!("[stderr] {line}");
                if let Some(tx) = &stderr_tx {
                    let _ = tx.send(tagged.clone());
                }
                collected.push(tagged);
            }
            collected
        });

        let mut logs = Vec::new();
        let mut result: Option<Result<Value, ExecError>> = None;
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<WorkerEvent>(&line) {
                Ok(WorkerEvent::Log { line }) => {
                    if let Some(tx) = &log_tx {
                        let _ = tx.send(line.clone());
                    }
                    logs.push(line);
                }
                Ok(WorkerEvent::Result { value }) => result = Some(Ok(value)),
                Ok(WorkerEvent::Error { message, trace }) => {
                    result = Some(Err(ExecError::Workload { message, trace }))
                }
                Err(e) => return Err(ExecError::Protocol(format!("{e}: {line}"))),
            }
        }

        if let Ok(mut stderr_logs) = stderr_task.await {
            logs.append(&mut stderr_logs);
        }
        match result {
            Some(Ok(value)) => Ok((value, logs)),
            Some(Err(e)) => Err(e),
            None => Err(ExecError::NoResult),
        }
    }

}

/// Write the job code into a fresh scratch dir; returns the dir guard and
/// the file name for the runtime (`job.py` / `job.ts` / `job.mjs`).
fn write_job_dir(req: &ExecRequest) -> Result<(tempfile::TempDir, &'static str), ExecError> {
    let job_dir = tempfile::Builder::new().prefix("cortex-job-").tempdir()?;
    let job_file = match req.runtime {
        Runtime::Python => "job.py",
        Runtime::Typescript => "job.ts",
        Runtime::Javascript => "job.mjs",
    };
    std::fs::write(job_dir.path().join(job_file), &req.code)?;
    Ok((job_dir, job_file))
}

fn parse_event(line: &str) -> Result<WorkerEvent, ExecError> {
    serde_json::from_str(line).map_err(|e| ExecError::Protocol(format!("{e}: {line}")))
}

/// SIGKILLing the engine client on timeout can orphan the container/microVM;
/// tell the engine to kill it by name, best-effort.
async fn kill_container(plan: &LaunchPlan) {
    if let Some(name) = &plan.container_name {
        let _ = Command::new(&plan.program)
            .args(["kill", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(runtime: Runtime, code: &str) -> ExecRequest {
        ExecRequest {
            runtime,
            code: code.to_string(),
            params: json!({"n": 4}),
            inputs: json!({}),
            timeout_secs: 30,
        }
    }

    #[tokio::test]
    async fn python_workload_returns_value_and_logs() {
        let exec = Executor::new().unwrap();
        let code = r#"
def handler(params, inputs):
    print("crunching", params["n"], "items")
    return {"doubled": params["n"] * 2}
"#;
        let out = exec.execute(req(Runtime::Python, code), None).await.unwrap();
        assert_eq!(out.value, json!({"doubled": 8}));
        assert!(out.logs.iter().any(|l| l.contains("crunching")));
    }

    #[tokio::test]
    async fn javascript_workload_runs() {
        let exec = Executor::new().unwrap();
        let code = r#"
export async function handler(params, inputs) {
  console.log("hello from node");
  return { tripled: params.n * 3 };
}
"#;
        let out = exec
            .execute(req(Runtime::Javascript, code), None)
            .await
            .unwrap();
        assert_eq!(out.value, json!({"tripled": 12}));
        assert!(out.logs.iter().any(|l| l.contains("hello from node")));
    }

    #[tokio::test]
    async fn python_error_is_reported() {
        let exec = Executor::new().unwrap();
        let code = "def handler(params, inputs):\n    raise ValueError('boom')\n";
        let err = exec
            .execute(req(Runtime::Python, code), None)
            .await
            .unwrap_err();
        match err {
            ExecError::Workload { message, .. } => assert!(message.contains("boom")),
            other => panic!("expected workload error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_kills_runaway_workload() {
        let exec = Executor::new().unwrap();
        let mut r = req(
            Runtime::Python,
            "import time\ndef handler(params, inputs):\n    time.sleep(60)\n",
        );
        r.timeout_secs = 1;
        let err = exec.execute(r, None).await.unwrap_err();
        assert!(matches!(err, ExecError::Timeout(1)));
    }

    #[tokio::test]
    async fn pool_reuses_warm_interpreters() {
        let exec = Executor::new().unwrap();
        let pool = exec.pool.as_ref().expect("pool on by default in process mode");
        assert_eq!(pool.idle_count(PoolKind::Python), 0);

        let code = "def handler(params, inputs):\n    return params['n']\n";
        exec.execute(req(Runtime::Python, code), None).await.unwrap();
        assert_eq!(pool.idle_count(PoolKind::Python), 1, "worker parked after job");

        exec.execute(req(Runtime::Python, code), None).await.unwrap();
        assert_eq!(
            pool.idle_count(PoolKind::Python),
            1,
            "second job reused the warm worker instead of spawning"
        );
    }

    #[tokio::test]
    async fn pool_keeps_worker_after_workload_error() {
        let exec = Executor::new().unwrap();
        let pool = exec.pool.as_ref().unwrap();
        let bad = "def handler(params, inputs):\n    raise ValueError('nope')\n";
        let good = "def handler(params, inputs):\n    return 'ok'\n";

        assert!(exec.execute(req(Runtime::Python, bad), None).await.is_err());
        assert_eq!(pool.idle_count(PoolKind::Python), 1, "clean failure keeps worker");

        let out = exec.execute(req(Runtime::Python, good), None).await.unwrap();
        assert_eq!(out.value, json!("ok"));
    }

    #[tokio::test]
    async fn pool_serves_js_and_ts_from_one_node_pool() {
        let exec = Executor::new().unwrap();
        let pool = exec.pool.as_ref().unwrap();
        let js = "export const handler = (p) => p.n * 2;\n";
        let ts = "export const handler = (p: { n: number }): number => p.n * 3;\n";

        let a = exec.execute(req(Runtime::Javascript, js), None).await.unwrap();
        assert_eq!(a.value, json!(8));
        assert_eq!(pool.idle_count(PoolKind::Node), 1);

        let b = exec.execute(req(Runtime::Typescript, ts), None).await.unwrap();
        assert_eq!(b.value, json!(12));
        assert_eq!(pool.idle_count(PoolKind::Node), 1, "TS reused the JS worker");
    }

    #[tokio::test]
    async fn logs_stream_during_execution() {
        let exec = Executor::new().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let code = "def handler(params, inputs):\n    print('line-1')\n    print('line-2')\n    return None\n";
        exec.execute(req(Runtime::Python, code), Some(tx)).await.unwrap();
        let mut streamed = Vec::new();
        while let Ok(line) = rx.try_recv() {
            streamed.push(line);
        }
        assert_eq!(streamed, vec!["line-1", "line-2"]);
    }
}
